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
                    cp.set_pipeline(&fill_pipe);
                    cp.set_bind_group(0, &fill_bind, &[]);
                    cp.dispatch_workgroups(groups, 1, 1);
                    cp.set_pipeline(&verify_pipe);
                    cp.set_bind_group(0, &verify_bind, &[]);
                    cp.dispatch_workgroups(groups, 1, 1);
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

    /// Measure sustained memory bandwidth (GB/s) over ~`target_ms` by streaming
    /// a large VRAM buffer (read+write each element). Used to find the GDDR6
    /// *effective-bandwidth peak* — past it, ECC correction eats the gains.
    pub fn measure_bandwidth_gbps(&self, target_ms: u64) -> f64 {
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

        let start = std::time::Instant::now();
        let mut passes: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
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
                return 0.0;
            }
        }
        self.device.poll(wgpu::Maintain::Wait);
        let secs = start.elapsed().as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        // 8 bytes moved per element per pass (read + write).
        let bytes_moved = passes as f64 * len as f64 * 8.0;
        bytes_moved / secs / 1e9
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
