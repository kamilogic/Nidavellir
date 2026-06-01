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
const HASH2: u32 = 40503;
const TABLE_INIT: u32 = 2246822519;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    a: u32,
    n: u32,
    _pad: [u32; 2],
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

const MEM_SHADER: &str = r#"
struct P { gathers: u32, n: u32, p0: u32, p1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<storage, read> table: array<u32>;
@group(0) @binding(2) var<uniform> p: P;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var acc: u32 = 0u;
    for (var k: u32 = 0u; k < p.gathers; k = k + 1u) {
        let idx = (i * 2654435761u + k * 40503u) % p.n;
        acc = acc + table[idx];
    }
    data[i] = acc;
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

/// A live GPU device for running the battery (set up once, reused per stage).
pub struct GpuCtx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub adapter_name: String,
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
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("nidavellir-gpu-stress"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
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
        Ok(Self { device, queue, adapter_name, crashed })
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

    /// Sustained memory-bound known-answer test: many pseudo-random gathers from
    /// a table, dispatched back-to-back for ~`target_ms` (idempotent, so the CPU
    /// reference is computed once regardless of how many dispatches ran).
    pub fn run_memory(&self, name: &str, elements: u32, gathers: u32, target_ms: u64) -> StageReport {
        let start = std::time::Instant::now();
        let table: Vec<u32> = (0..elements).map(|j| j.wrapping_mul(TABLE_INIT)).collect();
        let byte_size = (elements as usize * 4) as u64;

        let data = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mem-data"),
            size: byte_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let table_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mem-table"),
            contents: bytemuck::cast_slice(&table),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mem-params"),
            contents: bytemuck::bytes_of(&Params { a: gathers, n: elements, _pad: [0; 2] }),
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
                wgpu::BindGroupEntry { binding: 1, resource: table_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });

        let groups = elements.div_ceil(64);
        let mut k: u64 = 0;
        while (start.elapsed().as_millis() as u64) < target_ms {
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("mem"),
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

        let expected: Vec<u32> = (0..elements)
            .map(|i| {
                let mut acc = 0u32;
                for k in 0..gathers {
                    let idx = (i.wrapping_mul(HASH1).wrapping_add(k.wrapping_mul(HASH2))) % elements;
                    acc = acc.wrapping_add(table[idx as usize]);
                }
                acc
            })
            .collect();
        let (mismatches, mapped_ok) = self.readback_compare(&data, byte_size, &expected);
        StageReport {
            name: name.to_string(),
            result: self.verdict(mismatches, mapped_ok),
            mismatches,
            elapsed_ms: start.elapsed().as_millis() as u64,
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
