//! Real GPU compute-validation battery (wgpu / Vulkan).
//!
//! Detects instability by **wrong results** (silent errors), not just crashes
//! (roadmap §12). The battery covers distinct failure modes so it doesn't need
//! a specific game to surface them:
//!   - **ALU** — known-answer LCG, heavy integer mul/add → core stability.
//!   - **Memory** — many pseudo-random gathers from a table → memory subsystem.
//!   - **Burst** — the ALU load in on/off bursts → dI/dt power transients
//!     (what bursty game frametimes hit and a steady stress misses).
//!
//! Every stage has a bit-exact CPU reference; any divergence ⇒ `SilentError`,
//! a device-lost ⇒ `Crash`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nidavellir_core::gpu_sweep::StabilityResult;
use wgpu::util::DeviceExt;

const C1: u32 = 1664525;
const C2: u32 = 1013904223;
const HASH1: u32 = 2654435761;
const TABLE_INIT: u32 = 2246822519;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    a: u32,
    n: u32,
    _pad: [u32; 2],
}

/// Generic 4×u32 uniform block (16-byte aligned) reused by several kernels.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Quad {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
}

/// Result of one battery stage.
#[derive(Debug, Clone)]
pub struct StageReport {
    pub name: String,
    pub result: StabilityResult,
    pub mismatches: u32,
    pub elapsed_ms: u64,
}

/// Result of a render-pipeline run: the stability verdict plus the rendered
/// frame count and FPS (the benchmark's performance metric).
#[derive(Debug, Clone, Copy)]
pub struct RenderResult {
    pub result: StabilityResult,
    pub frames: u64,
    pub fps: f64,
}

const ALU_SHADER: &str = r#"
struct P { iters: u32, n: u32, p0: u32, p1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var x = data[i];
    for (var k: u32 = 0u; k < p.iters; k = k + 1u) {
        x = x * 1664525u + 1013904223u;
    }
    data[i] = x;
}
"#;

// Max-power ALU virus: each lane runs FOUR independent LCG chains in a vec4
// (4× instruction-level parallelism → saturates the ALU pipelines for the
// highest sustained power draw, near a real power-virus, so high-voltage points
// actually reach the power cap). Each vec4 lane is a pure LCG of its own seed →
// fully verifiable via jump-ahead. No memory traffic (registers only) so power,
// not bandwidth, is the limit.
const POWER_SHADER: &str = r#"
struct P { iters: u32, n: u32, p0: u32, p1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<vec4<u32>>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var v = data[i];
    for (var k: u32 = 0u; k < p.iters; k = k + 1u) {
        v = v * 1664525u + vec4<u32>(1013904223u);
    }
    data[i] = v;
}
"#;

// Each lane gathers from a LARGE (VRAM-resident) table, striding across the
// whole table so accesses miss cache and hit the memory controller / DRAM —
// the path that shares the core voltage rail.
const MEM_SHADER: &str = r#"
struct P { gathers: u32, lanes: u32, table_len: u32, p1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<storage, read> table: array<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.lanes) { return; }
    var acc: u32 = 0u;
    for (var k: u32 = 0u; k < p.gathers; k = k + 1u) {
        let idx = (i * 2654435761u + k * 2246822519u) % p.table_len;
        acc = acc + table[idx];
    }
    data[i] = acc;
}
"#;

// Fills the large table on the GPU so we don't upload hundreds of MB. Uses a
// grid-stride loop so a bounded workgroup count covers a huge buffer (the X
// dispatch dimension is capped at 65535).
const FILL_SHADER: &str = r#"
struct P { n: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read_write> table: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = gid.x;
    loop {
        if (i >= p.n) { break; }
        table[i] = i * 2246822519u;
        i = i + stride;
    }
}
"#;

// Memory-bandwidth kernel: read+write every element (8 bytes moved/elem) with
// minimal compute → DRAM-bandwidth bound. Grid-stride for huge buffers.
const BW_SHADER: &str = r#"
struct P { n: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read_write> buf: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = gid.x;
    loop {
        if (i >= p.n) { break; }
        buf[i] = buf[i] * 1664525u + 1013904223u;
        i = i + stride;
    }
}
"#;

// Fused core+memory kernel: each lane runs heavy LCG rounds (core ALU) AND, in
// the SAME loop, streams a large VRAM buffer with a read-modify-write (memory
// bandwidth) — so the shader cores and the memory controller are loaded
// *simultaneously per invocation*, like a real game shader. (Separate ALU and
// bandwidth dispatches in one pass run sequentially and don't co-load: the
// memory dispatch just adds bubbles that lower utilization.) `data[i]` is a
// pure LCG of its seed (verifiable via jump-ahead); the `buf` RMW is load-only.
const FUSED_SHADER: &str = r#"
struct P { iters: u32, n: u32, buf_n: u32, pad: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<storage, read_write> buf: array<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var x = data[i];
    for (var k: u32 = 0u; k < p.iters; k = k + 1u) {
        x = x * 1664525u + 1013904223u;
        let idx = (i * 2654435761u + k * 2246822519u) % p.buf_n;
        buf[idx] = buf[idx] + x;
    }
    data[i] = x;
}
"#;

// Pointer-chasing chain: chain[i] = (i*CP + CQ) & mask — a permutation of a
// power-of-two-sized buffer. Following it does data-dependent random reads
// (memory-latency bound) where ANY uncorrected error sends the chase down a
// wrong address and cascades into a totally different result — far more
// sensitive to memory/addressing faults than a linear read/verify (which the
// GDDR6 link CRC tends to mask).
const CHAIN_CP: u32 = 2654435761;
const CHAIN_CQ: u32 = 1442695041;

const CHAIN_FILL_SHADER: &str = r#"
struct P { n: u32, mask: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read_write> chain: array<u32>;
@group(0) @binding(1) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = gid.x;
    loop {
        if (i >= p.n) { break; }
        chain[i] = (i * 2654435761u + 1442695041u) & p.mask;
        i = i + stride;
    }
}
"#;

const CHASE_SHADER: &str = r#"
struct P { steps: u32, mask: u32, lanes: u32, p2: u32 };
@group(0) @binding(0) var<storage, read> chain: array<u32>;
@group(0) @binding(1) var<storage, read_write> out: array<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let lane = gid.x;
    if (lane >= p.lanes) { return; }
    var idx = (lane * 2654435761u + 1442695041u) & p.mask;
    for (var k: u32 = 0u; k < p.steps; k = k + 1u) {
        idx = chain[idx] & p.mask;
    }
    out[lane] = idx;
}
"#;

// Real RENDER-pipeline stress (not compute): a heavy fragment shader over many
// overlapping instanced triangles (overdraw → raster + ROP + TMU + fragment ALU,
// the game path compute never touches). The output is a deterministic function
// of pixel position + instance, so a stable GPU renders the SAME frame every
// time; a diverging frame checksum is a silent error before a hard crash.
const RENDER_SHADER: &str = r#"
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) seed: f32 };
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let a = f32(ii) * 0.137;
    let s = sin(a); let c = cos(a);
    let v = p[vi];
    let r = vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
    var o: VOut;
    o.pos = vec4<f32>(r * 0.999, 0.0, 1.0);
    o.uv = (v + vec2<f32>(1.0, 1.0)) * 0.5;
    o.seed = f32(ii);
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    var x = in.uv.x * 64.0 + in.seed;
    var y = in.uv.y * 64.0 - in.seed;
    var acc = 0.0;
    for (var k = 0; k < 320; k = k + 1) {
        acc = acc + sin(x) * cos(y) + tanh(x * 0.01);
        x = fma(x, 1.0001, 0.013);
        y = fma(y, 0.9997, 0.017);
    }
    let v = fract(abs(acc) * 0.00137);
    return vec4<f32>(v, fract(v * 7.0), fract(v * 13.0), 1.0);
}
"#;

// Sum every texel (as u32) into one atomic — a whole-frame checksum read back as
// 4 bytes, so frame-to-frame determinism is cheap to verify.
const REDUCE_SHADER: &str = r#"
struct P { n: u32, p0: u32, p1: u32, p2: u32 };
@group(0) @binding(0) var<storage, read> buf: array<u32>;
@group(0) @binding(1) var<storage, read_write> res: atomic<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {
    let stride = nwg.x * 64u;
    var i = g.x; var acc = 0u;
    loop { if (i >= p.n) { break; } acc = acc + buf[i]; i = i + stride; }
    atomicAdd(&res, acc);
}
"#;

// VRAM integrity: write a deterministic pattern, then verify it bit-for-bit,
// counting mismatches on the GPU (only the count is read back).
const VRAM_PATTERN_FN: &str = r#"
fn pattern(i: u32, mode: u32) -> u32 {
    if (mode == 0u) { return i * 2654435761u; }       // address-in-cell
    if (mode == 1u) { return 0xffffffffu; }           // all ones
    if (mode == 2u) { return 0u; }                    // all zeros
    if (mode == 3u) { if ((i & 1u) == 0u) { return 0xaaaaaaaau; } return 0x55555555u; } // checkerboard
    return 1u << (i % 32u);                            // walking bit
}
"#;

#[cfg(test)]
fn lcg(seed: u32, iters: u32) -> u32 {
    let mut x = seed;
    for _ in 0..iters {
        x = x.wrapping_mul(C1).wrapping_add(C2);
    }
    x
}

/// f(x) = C1*x + C2 (mod 2^32). Returns (A, C) for f^n via fast exponentiation,
/// so `f^n(seed) = A*seed + C` — lets us verify any number of GPU rounds in
/// O(log n) on the CPU instead of replaying the loop.
fn lcg_pow(n: u64) -> (u32, u32) {
    // Compose: apply `h` then `g` → g(h(x)) = (g.A*h.A)*x + (g.A*h.C + g.C).
    fn compose(h: (u32, u32), g: (u32, u32)) -> (u32, u32) {
        (g.0.wrapping_mul(h.0), g.0.wrapping_mul(h.1).wrapping_add(g.1))
    }
    let mut result = (1u32, 0u32); // identity
    let mut base = (C1, C2); // f^1
    let mut e = n;
    while e > 0 {
        if e & 1 == 1 {
            result = compose(result, base);
        }
        base = compose(base, base);
        e >>= 1;
    }
    result
}

#[cfg(test)]
fn lcg_jump(seed: u32, n: u64) -> u32 {
    let (a, c) = lcg_pow(n);
    a.wrapping_mul(seed).wrapping_add(c)
}

/// (A, C) for `f^n` where f(x) = (CHAIN_CP*x + CHAIN_CQ) mod (mask+1), so the
/// pointer-chase result is `(A*start + C) & mask` — verified in O(log n).
fn affine_pow_mod(n: u64, mask: u32) -> (u32, u32) {
    fn compose(h: (u32, u32), g: (u32, u32), mask: u32) -> (u32, u32) {
        (
            g.0.wrapping_mul(h.0) & mask,
            (g.0.wrapping_mul(h.1).wrapping_add(g.1)) & mask,
        )
    }
    let mut result = (1u32 & mask, 0u32);
    let mut base = (CHAIN_CP & mask, CHAIN_CQ & mask);
    let mut e = n;
    while e > 0 {
        if e & 1 == 1 {
            result = compose(result, base, mask);
        }
        base = compose(base, base, mask);
        e >>= 1;
    }
    result
}

/// A live GPU device for running the battery (set up once, reused per stage).
pub struct GpuCtx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub adapter_name: String,
    /// Largest single storage buffer we may allocate (bytes).
    pub max_buffer_bytes: u64,
    crashed: Arc<AtomicBool>,
}

impl GpuCtx {
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "no suitable GPU adapter found".to_string())?;
        let adapter_name = adapter.get_info().name;
        // Request the adapter's full limits so we can allocate large
        // VRAM-resident buffers (cache-busting + VRAM coverage).
        let limits = adapter.limits();
        let max_buffer_bytes = limits.max_storage_buffer_binding_size as u64;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("nidavellir-gpu-stress"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
            },
            None,
        ))
        .map_err(|e| format!("request_device failed: {e}"))?;

        let crashed = Arc::new(AtomicBool::new(false));
        {
            let crashed = crashed.clone();
            device.on_uncaptured_error(Box::new(move |e| {
                eprintln!("wgpu device error: {e}");
                crashed.store(true, Ordering::SeqCst);
            }));
        }
        Ok(Self { device, queue, adapter_name, max_buffer_bytes, crashed })
    }

    fn verdict(&self, mismatches: u32, mapped_ok: bool) -> StabilityResult {
        if !mapped_ok || self.crashed.load(Ordering::SeqCst) {
            StabilityResult::Crash
        } else if mismatches == 0 {
            StabilityResult::Stable
        } else {
            StabilityResult::SilentError
        }
    }

    /// Sustained ALU known-answer test: dispatches the LCG kernel back-to-back
    /// for ~`target_ms`, keeping the GPU **saturated** (the buffer accumulates,
    /// so after K dispatches every lane has had `iters*K` LCG rounds). Verified
    /// via LCG jump-ahead, so the CPU reference is O(log n) regardless of load.
    pub fn run_alu(&self, name: &str, elements: u32, iters: u32, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();
        let input: Vec<u32> = (0..elements).collect();
        let byte_size = (elements as usize * 4) as u64;

        let data = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("alu-data"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("alu-params"),
            contents: bytemuck::bytes_of(&Params { a: iters, n: elements, _pad: [0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("alu"),
            source: wgpu::ShaderSource::Wgsl(ALU_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("alu"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("alu"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            ],
        });

        let groups = elements.div_ceil(64);
        let mut k: u64 = 0;
        // Keep the queue fed back-to-back; bound depth with an occasional wait.
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("alu"),
                    timestamp_writes: None,
                });
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // expected[i] = f^(iters*k)(i) where f(x)=a*x+c, via fast exponentiation.
        let (a, c) = lcg_pow((iters as u64).wrapping_mul(k));
        let expected: Vec<u32> = (0..elements).map(|i| a.wrapping_mul(i).wrapping_add(c)).collect();
        let (mismatches, mapped_ok) = self.readback_compare(&data, byte_size, &expected);
        StageReport {
            name: name.to_string(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Sustained MAX-POWER ALU load (4-way ILP per lane via `vec4`), known-answer
    /// checked. The densest, most power-hungry load we have — used by the power
    /// sweep so high-voltage points reach the power cap and the V↔W map is real.
    pub fn run_power_load(&self, elements: u32, iters: u32, target_ms: u64) -> StageReport {
        const S1: u32 = 0x9e3779b9;
        const S2: u32 = 0x85ebca6b;
        const S3: u32 = 0xc2b2ae35;
        let start = std::time::Instant::now();

        // Four independent LCG seeds per lane, packed as vec4.
        let mut input: Vec<u32> = Vec::with_capacity(elements as usize * 4);
        for i in 0..elements {
            input.push(i);
            input.push(i ^ S1);
            input.push(i ^ S2);
            input.push(i ^ S3);
        }
        let byte_size = (elements as usize * 16) as u64;

        let data = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pwr-data"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pwr-params"),
            contents: bytemuck::bytes_of(&Params { a: iters, n: elements, _pad: [0; 2] }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pwr"),
            source: wgpu::ShaderSource::Wgsl(POWER_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pwr"), layout: None, module: &module, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pwr"), layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            ],
        });

        let groups = elements.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        let (a, c) = lcg_pow((iters as u64).wrapping_mul(k));
        let mut expected: Vec<u32> = Vec::with_capacity(elements as usize * 4);
        for i in 0..elements {
            expected.push(a.wrapping_mul(i).wrapping_add(c));
            expected.push(a.wrapping_mul(i ^ S1).wrapping_add(c));
            expected.push(a.wrapping_mul(i ^ S2).wrapping_add(c));
            expected.push(a.wrapping_mul(i ^ S3).wrapping_add(c));
        }
        let (mismatches, mapped_ok) = self.readback_compare(&data, byte_size, &expected);
        StageReport {
            name: "PowerLoad".into(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Sustained memory-bound known-answer test. `lanes` output lanes each gather
    /// `gathers` times from a LARGE (~256 MB) VRAM-resident table, striding so the
    /// reads miss cache and traverse the memory controller — the path on the core
    /// voltage rail. Idempotent → CPU reference computed once (table values via
    /// the same on-the-fly hash, so it isn't stored on the CPU).
    pub fn run_memory(&self, name: &str, lanes: u32, gathers: u32, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();

        // Large table sized to ~256 MB (capped by device limits), cache-busting.
        let target_table_bytes = 256u64 * 1024 * 1024;
        let table_bytes = target_table_bytes.min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let table_len = (table_bytes / 4) as u32;
        let out_bytes = (lanes as usize * 4) as u64;

        let table = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mem-table"),
            size: (table_len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let data = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mem-data"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Fill the table on the GPU (avoids a 256 MB upload).
        let fill_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fill-params"),
            contents: bytemuck::bytes_of(&Quad { a: table_len, b: 0, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let fill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fill"),
            source: wgpu::ShaderSource::Wgsl(FILL_SHADER.into()),
        });
        let fill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fill"),
            layout: None,
            module: &fill_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let fill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fill"),
            layout: &fill_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fill_params.as_entire_binding() },
            ],
        });
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&fill_pipe);
                cp.set_bind_group(0, &fill_bind, &[]);
                cp.dispatch_workgroups(table_len.div_ceil(64).min(65535), 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }

        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mem-params"),
            contents: bytemuck::bytes_of(&Quad { a: gathers, b: lanes, c: table_len, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mem"),
            source: wgpu::ShaderSource::Wgsl(MEM_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("mem"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mem"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });

        let groups = lanes.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // table[idx] = idx * TABLE_INIT (matches FILL_SHADER) — computed inline.
        let expected: Vec<u32> = (0..lanes)
            .map(|i| {
                let mut acc = 0u32;
                for kk in 0..gathers {
                    let idx = (i.wrapping_mul(HASH1).wrapping_add(kk.wrapping_mul(TABLE_INIT))) % table_len;
                    acc = acc.wrapping_add(idx.wrapping_mul(TABLE_INIT));
                }
                acc
            })
            .collect();
        let (mismatches, mapped_ok) = self.readback_compare(&data, out_bytes, &expected);
        StageReport {
            name: name.to_string(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// VRAM integrity check (roadmap §12 Phase 1): allocate a large VRAM buffer,
    /// write/verify deterministic patterns (address-in-cell, all-1/0,
    /// checkerboard, walking-bit), counting mismatches on the GPU. Run at stock
    /// before tuning — a failure here means the memory itself is unstable.
    pub fn run_vram_check(&self, target_bytes: u64, passes: u32) -> StageReport {
        let start = std::time::Instant::now();
        let bytes = target_bytes.min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let len = (bytes / 4) as u32;

        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vram-buf"),
            size: (len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let result = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vram-result"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fill_src = format!(
            "{VRAM_PATTERN_FN}\nstruct P {{ mode: u32, n: u32, p0: u32, p1: u32 }};\n\
             @group(0) @binding(0) var<storage, read_write> buf: array<u32>;\n\
             @group(0) @binding(1) var<uniform> p: P;\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n\
               let stride = nwg.x * 64u; var i = g.x; loop {{ if (i >= p.n) {{ break; }} buf[i] = pattern(i, p.mode); i = i + stride; }}\n}}"
        );
        let verify_src = format!(
            "{VRAM_PATTERN_FN}\nstruct P {{ mode: u32, n: u32, p0: u32, p1: u32 }};\n\
             @group(0) @binding(0) var<storage, read> buf: array<u32>;\n\
             @group(0) @binding(1) var<uniform> p: P;\n\
             @group(0) @binding(2) var<storage, read_write> res: atomic<u32>;\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n\
               let stride = nwg.x * 64u; var i = g.x; loop {{ if (i >= p.n) {{ break; }} if (buf[i] != pattern(i, p.mode)) {{ atomicAdd(&res, 1u); }} i = i + stride; }}\n}}"
        );
        let fill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vram-fill"),
            source: wgpu::ShaderSource::Wgsl(fill_src.into()),
        });
        let verify_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vram-verify"),
            source: wgpu::ShaderSource::Wgsl(verify_src.into()),
        });
        let fill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vram-fill"),
            layout: None,
            module: &fill_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let verify_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("vram-verify"),
            layout: None,
            module: &verify_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });

        let groups = len.div_ceil(64).min(65535);
        // Repeat write+read cycles per pattern so the memory is actually
        // hammered at its clock (a single fill→verify is too quick to surface
        // marginal instability — exactly why a too-high mem OC used to pass).
        const VRAM_REPS: u32 = 10;
        let mut total_mismatches = 0u32;
        'outer: for _ in 0..passes.max(1) {
            for mode in 0u32..5 {
                let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vram-params"),
                    contents: bytemuck::bytes_of(&Quad { a: mode, b: len, c: 0, d: 0 }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let fill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vram-fill"),
                    layout: &fill_pipe.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
                    ],
                });
                let verify_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vram-verify"),
                    layout: &verify_pipe.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: result.as_entire_binding() },
                    ],
                });
                self.queue.write_buffer(&result, 0, &0u32.to_ne_bytes());
                let mut enc = self.device.create_command_encoder(&Default::default());
                {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    for _ in 0..VRAM_REPS {
                        cp.set_pipeline(&fill_pipe);
                        cp.set_bind_group(0, &fill_bind, &[]);
                        cp.dispatch_workgroups(groups, 1, 1);
                        cp.set_pipeline(&verify_pipe);
                        cp.set_bind_group(0, &verify_bind, &[]);
                        cp.dispatch_workgroups(groups, 1, 1);
                    }
                }
                self.queue.submit(Some(enc.finish()));
                self.device.poll(wgpu::Maintain::Wait);
                total_mismatches = total_mismatches.saturating_add(self.read_u32(&result));
                if self.crashed.load(Ordering::SeqCst) {
                    break 'outer;
                }
            }
        }

        StageReport {
            name: format!("VRAM ({} MB)", bytes / (1024 * 1024)),
            result: self.verdict(total_mismatches, !self.crashed.load(Ordering::SeqCst)),
            mismatches: total_mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Pointer-chasing memory test: data-dependent random reads through a chain
    /// in a large VRAM buffer, sustained for ~`target_ms`. Catches uncorrected
    /// memory/addressing errors that a linear read/verify misses (a wrong read
    /// derails the whole chase). Returns SilentError on any divergence.
    pub fn run_mem_chase(&self, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();
        let want_bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        // Chain length must be a power of two (mask = len-1).
        let mut len: u32 = 1 << 26; // 64M = 256 MB
        while (len as u64) * 4 > want_bytes && len > (1 << 20) {
            len >>= 1;
        }
        let mask = len - 1;
        let lanes: u32 = 65_536;
        let steps: u32 = 8_192;

        let chain = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chain"),
            size: (len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chase-out"),
            size: (lanes as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        // Fill the chain.
        let fill_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chain-fill-params"),
            contents: bytemuck::bytes_of(&Quad { a: len, b: mask, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let fill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chain-fill"),
            source: wgpu::ShaderSource::Wgsl(CHAIN_FILL_SHADER.into()),
        });
        let fill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("chain-fill"),
            layout: None,
            module: &fill_mod,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let fill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chain-fill"),
            layout: &fill_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fill_params.as_entire_binding() },
            ],
        });
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&fill_pipe);
                cp.set_bind_group(0, &fill_bind, &[]);
                cp.dispatch_workgroups(len.div_ceil(64).min(65535), 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }

        // Chase.
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chase-params"),
            contents: bytemuck::bytes_of(&Quad { a: steps, b: mask, c: lanes, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chase"),
            source: wgpu::ShaderSource::Wgsl(CHASE_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("chase"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chase"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: out.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });

        let groups = lanes.div_ceil(64);
        let mut k = 0u64;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // Reference: out[lane] = f^steps(start_lane), start_lane = (lane*CP+CQ)&mask.
        let (a, c) = affine_pow_mod(steps as u64, mask);
        let expected: Vec<u32> = (0..lanes)
            .map(|lane| {
                let s = (lane.wrapping_mul(CHAIN_CP).wrapping_add(CHAIN_CQ)) & mask;
                (a.wrapping_mul(s).wrapping_add(c)) & mask
            })
            .collect();
        let (mismatches, mapped_ok) = self.readback_compare(&out, (lanes as u64) * 4, &expected);
        StageReport {
            name: format!("Mem chase ({} MB)", (len as u64 * 4) / (1024 * 1024)),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Combined core+memory soak: hammers ALU (core), a **bandwidth-saturating**
    /// VRAM stream (memory controller / DRAM throughput), and a pointer-chase
    /// (memory latency/addressing) **at the same time** for ~`target_ms` —
    /// loading the shared voltage rail / power / thermals like a real game, the
    /// condition that exposes instability a single-axis test misses. ALU and
    /// chase are known-answer checked (SilentError on any divergence); the
    /// bandwidth stream is load-only.
    pub fn run_combined(&self, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();

        // --- Fused core+memory (ALU + bandwidth in one kernel) ---
        // Each lane does `fused_iters` LCG rounds (core) and a memory RMW into a
        // large VRAM buffer each round (bandwidth), co-loading both per dispatch.
        let fused_n: u32 = 1 << 20; // 1,048,576 lanes
        let fused_iters: u32 = 512; // LCG + memory RMW per lane per dispatch
        let buf_bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let buf_n = (buf_bytes / 4) as u32;
        let fused_data = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-fused-data"),
            contents: bytemuck::cast_slice(&(0..fused_n).collect::<Vec<u32>>()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let fused_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c-fused-buf"), size: (buf_n as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false,
        });
        let fused_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-fused-p"),
            contents: bytemuck::bytes_of(&Quad { a: fused_iters, b: fused_n, c: buf_n, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let alu_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c-fused"),
            source: wgpu::ShaderSource::Wgsl(FUSED_SHADER.into()),
        });
        let alu_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c-fused"), layout: None, module: &alu_mod, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let alu_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c-fused"), layout: &alu_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: fused_data.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: fused_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: fused_params.as_entire_binding() },
            ],
        });

        // --- Memory (pointer-chase) ---
        let clen: u32 = 1 << 24; // 64 MB chain
        let mask = clen - 1;
        let lanes: u32 = 65_536;
        let steps: u32 = 3_072;
        let chain = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c-chain"), size: (clen as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false,
        });
        let cout = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("c-out"), size: (lanes as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        let cfill_p = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-chain-fp"),
            contents: bytemuck::bytes_of(&Quad { a: clen, b: mask, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let cfill_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c-chain-fill"), source: wgpu::ShaderSource::Wgsl(CHAIN_FILL_SHADER.into()),
        });
        let cfill_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c-chain-fill"), layout: None, module: &cfill_mod, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let cfill_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c-chain-fill"), layout: &cfill_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cfill_p.as_entire_binding() },
            ],
        });
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            { let mut cp = enc.begin_compute_pass(&Default::default());
              cp.set_pipeline(&cfill_pipe); cp.set_bind_group(0, &cfill_bind, &[]);
              cp.dispatch_workgroups(clen.div_ceil(64).min(65535), 1, 1); }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }
        let chase_p = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("c-chase-p"),
            contents: bytemuck::bytes_of(&Quad { a: steps, b: mask, c: lanes, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let chase_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("c-chase"), source: wgpu::ShaderSource::Wgsl(CHASE_SHADER.into()),
        });
        let chase_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("c-chase"), layout: None, module: &chase_mod, entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let chase_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("c-chase"), layout: &chase_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: chain.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: cout.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: chase_p.as_entire_binding() },
            ],
        });

        // Run the fused core+memory kernel (co-loads cores + DRAM) alongside the
        // pointer-chase (memory latency/addressing) each iteration.
        let alu_groups = fused_n.div_ceil(64);
        let chase_groups = lanes.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&alu_pipe);
                cp.set_bind_group(0, &alu_bind, &[]);
                cp.dispatch_workgroups(alu_groups, 1, 1);
                cp.set_pipeline(&chase_pipe);
                cp.set_bind_group(0, &chase_bind, &[]);
                cp.dispatch_workgroups(chase_groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            k += 1;
            if k % 8 == 0 {
                self.device.poll(wgpu::Maintain::Wait);
            }
            if self.crashed.load(Ordering::SeqCst) {
                break;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);

        // Verify fused ALU (lcg^(iters*k)) and chase (affine^steps).
        let (aa, ac) = lcg_pow((fused_iters as u64).wrapping_mul(k));
        let alu_exp: Vec<u32> = (0..fused_n).map(|i| aa.wrapping_mul(i).wrapping_add(ac)).collect();
        let (am, mm) = self.readback_compare(&fused_data, (fused_n as u64) * 4, &alu_exp);
        let (ca, cc) = affine_pow_mod(steps as u64, mask);
        let chase_exp: Vec<u32> = (0..lanes)
            .map(|l| {
                let s = (l.wrapping_mul(CHAIN_CP).wrapping_add(CHAIN_CQ)) & mask;
                (ca.wrapping_mul(s).wrapping_add(cc)) & mask
            })
            .collect();
        let (cm, mok) = self.readback_compare(&cout, (lanes as u64) * 4, &chase_exp);

        let mismatches = am.saturating_add(cm);
        StageReport {
            name: "Combined (core+mem)".into(),
            result: self.verdict(mismatches, mm && mok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Real graphics-pipeline stress with a frame-determinism check. Renders a
    /// heavy, deterministic instanced scene (overdraw + fragment ALU) to an
    /// offscreen target for ~`target_ms`, checksumming the framebuffer ~4×/s. A
    /// stable GPU yields an identical checksum every time; a divergence is a
    /// SilentError, a device-lost is a Crash. Closer to a game than any compute
    /// test (raster + ROP + fragment units), so it catches instability that
    /// compute-only validation passes. Returns the verdict plus the rendered
    /// frame count / FPS — the benchmark uses the FPS as its performance metric.
    pub fn run_render_stress(&self, target_ms: u64) -> RenderResult {
        let start = std::time::Instant::now();
        let mut frames: u64 = 0;
        const DIM: u32 = 1024; // 1024*4 = 4096 B/row (256-aligned for copy)
        const INSTANCES: u32 = 64; // full-screen triangles → heavy overdraw

        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-target"),
            size: wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render"), source: wgpu::ShaderSource::Wgsl(RENDER_SHADER.into()),
        });
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render"), layout: None,
            vertex: wgpu::VertexState { module: &shader, entry_point: "vs", buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader, entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
        });

        // Readback: copy the frame to a buffer, reduce to one checksum u32.
        let px_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render-px"), size: (DIM as u64) * (DIM as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let sum_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render-sum"), size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let n = DIM * DIM;
        let red_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render-rp"), contents: bytemuck::bytes_of(&Quad { a: n, b: 0, c: 0, d: 0 }), usage: wgpu::BufferUsages::UNIFORM,
        });
        let red_mod = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reduce"), source: wgpu::ShaderSource::Wgsl(REDUCE_SHADER.into()),
        });
        let red_pipe = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reduce"), layout: None, module: &red_mod, entry_point: "main", compilation_options: Default::default(),
        });
        let red_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reduce"), layout: &red_pipe.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: px_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sum_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: red_params.as_entire_binding() },
            ],
        });

        let mut reference: Option<u32> = None;
        let mut diverged = false;
        let mut last_check = std::time::Instant::now();

        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view, resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
                });
                rp.set_pipeline(&pipeline);
                rp.draw(0..3, 0..INSTANCES);
            }
            self.queue.submit(Some(enc.finish()));
            frames += 1;

            if self.crashed.load(Ordering::SeqCst) {
                break;
            }

            if last_check.elapsed().as_millis() >= 250 {
                last_check = std::time::Instant::now();
                let mut enc = self.device.create_command_encoder(&Default::default());
                enc.copy_texture_to_buffer(
                    wgpu::ImageCopyTexture { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    wgpu::ImageCopyBuffer { buffer: &px_buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(DIM * 4), rows_per_image: Some(DIM) } },
                    wgpu::Extent3d { width: DIM, height: DIM, depth_or_array_layers: 1 },
                );
                enc.clear_buffer(&sum_buf, 0, None);
                {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    cp.set_pipeline(&red_pipe); cp.set_bind_group(0, &red_bind, &[]);
                    cp.dispatch_workgroups(256, 1, 1);
                }
                self.queue.submit(Some(enc.finish()));
                self.device.poll(wgpu::Maintain::Wait);
                let sum = self.read_u32(&sum_buf);
                match reference {
                    None => reference = Some(sum),
                    Some(r) => {
                        if sum != r {
                            diverged = true;
                            break;
                        }
                    }
                }
            }
            self.device.poll(wgpu::Maintain::Wait);
        }

        let result = if self.crashed.load(Ordering::SeqCst) {
            StabilityResult::Crash
        } else if diverged {
            StabilityResult::SilentError
        } else {
            StabilityResult::Stable
        };
        let secs = start.elapsed().as_secs_f64().max(0.001);
        RenderResult { result, frames, fps: frames as f64 / secs }
    }

    /// Measure sustained memory bandwidth (GB/s) over ~`target_ms` by streaming
    /// a large VRAM buffer (read+write each element). Used to find the GDDR6
    /// *effective-bandwidth peak* — past it, ECC correction eats the gains.
    pub fn measure_bandwidth_gbps(&self, target_ms: u64) -> f64 {
        self.measure_bandwidth_stats(target_ms).0
    }

    /// Bandwidth over sub-windows → `(peak, min)` GB/s. A large gap between peak
    /// and min means the clock is *inconsistent* (GDDR6 CRC retries / throttle
    /// dips) — the real signal for the intermittent stutters a peak-only number
    /// hides.
    pub fn measure_bandwidth_stats(&self, target_ms: u64) -> (f64, f64) {
        let bytes = (256u64 * 1024 * 1024).min(self.max_buffer_bytes).max(64 * 1024 * 1024);
        let len = (bytes / 4) as u32;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bw-buf"),
            size: (len as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bw-params"),
            contents: bytemuck::bytes_of(&Quad { a: len, b: 0, c: 0, d: 0 }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bw"),
            source: wgpu::ShaderSource::Wgsl(BW_SHADER.into()),
        });
        let pipeline = self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bw"),
            layout: None,
            module: &module,
            entry_point: "main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bw"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
            ],
        });
        let groups = len.div_ceil(64).min(65535);

        // Warm-up pass (not timed).
        {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&Default::default());
                cp.set_pipeline(&pipeline);
                cp.set_bind_group(0, &bind, &[]);
                cp.dispatch_workgroups(groups, 1, 1);
            }
            self.queue.submit(Some(enc.finish()));
            self.device.poll(wgpu::Maintain::Wait);
        }

        // Measure several sub-windows and take the PEAK: the memory clock boosts
        // and throttles dynamically, so a single short window is noisy. The peak
        // reflects the bandwidth the clock can actually deliver (downward noise =
        // throttle dips, which we reject); it can't exceed real bandwidth.
        let window_ms: u64 = 500;
        let windows = (target_ms / window_ms).max(8);
        let mut peak_gbps = 0.0f64;
        let mut min_gbps = f64::INFINITY;
        for _ in 0..windows {
            let ws = std::time::Instant::now();
            let mut passes: u64 = 0;
            while (ws.elapsed().as_millis() as u64) < window_ms {
                let mut enc = self.device.create_command_encoder(&Default::default());
                {
                    let mut cp = enc.begin_compute_pass(&Default::default());
                    cp.set_pipeline(&pipeline);
                    cp.set_bind_group(0, &bind, &[]);
                    cp.dispatch_workgroups(groups, 1, 1);
                }
                self.queue.submit(Some(enc.finish()));
                passes += 1;
                if passes % 8 == 0 {
                    self.device.poll(wgpu::Maintain::Wait);
                }
                if self.crashed.load(Ordering::SeqCst) {
                    return (0.0, 0.0);
                }
            }
            self.device.poll(wgpu::Maintain::Wait);
            let secs = ws.elapsed().as_secs_f64();
            if secs > 0.0 {
                // 8 bytes moved per element per pass (read + write).
                let gbps = passes as f64 * len as f64 * 8.0 / secs / 1e9;
                if gbps > peak_gbps {
                    peak_gbps = gbps;
                }
                if gbps < min_gbps {
                    min_gbps = gbps;
                }
            }
        }
        if !min_gbps.is_finite() {
            min_gbps = peak_gbps;
        }
        (peak_gbps, min_gbps)
    }

    /// Read a single u32 from a COPY_SRC buffer.
    fn read_u32(&self, buffer: &wgpu::Buffer) -> u32 {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("u32-staging"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buffer, 0, &staging, 0, 4);
        self.queue.submit(Some(enc.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        if matches!(rx.recv(), Ok(Ok(()))) {
            let data = slice.get_mapped_range();
            u32::from_ne_bytes([data[0], data[1], data[2], data[3]])
        } else {
            0
        }
    }

    fn readback_compare(&self, buffer: &wgpu::Buffer, byte_size: u64, expected: &[u32]) -> (u32, bool) {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging"),
            size: byte_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(buffer, 0, &staging, 0, byte_size);
        self.queue.submit(Some(enc.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        let mapped_ok = matches!(rx.recv(), Ok(Ok(())));
        if !mapped_ok {
            return (0, false);
        }
        let data = slice.get_mapped_range();
        let output: &[u32] = bytemuck::cast_slice(&data);
        let mut mismatches = 0u32;
        for (got, exp) in output.iter().zip(expected.iter()) {
            if got != exp {
                mismatches += 1;
            }
        }
        (mismatches, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcg_reference_is_deterministic() {
        assert_eq!(lcg(0, 1), C2);
        assert_eq!(lcg(123, 1000), lcg(123, 1000));
    }

    #[test]
    fn lcg_jump_matches_loop() {
        for &(seed, n) in &[(0u32, 0u64), (0, 1), (123, 7), (999, 1000), (42, 65536)] {
            let mut x = seed;
            for _ in 0..n {
                x = x.wrapping_mul(C1).wrapping_add(C2);
            }
            assert_eq!(lcg_jump(seed, n), x, "seed={seed} n={n}");
        }
    }
}
