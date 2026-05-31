//! Real GPU compute-validation battery (wgpu / Vulkan).
//!
//! The point (roadmap §12): detect instability by **wrong results**, not just
//! crashes. Each kernel has a known answer computed identically on the CPU; any
//! bit divergence is a *silent error* — caught before a crash, which is exactly
//! what games hide and Afterburner never sees.
//!
//! Stage 1 here: the ALU-heavy known-answer test (LCG). Memory-bound and
//! burst-transient kernels follow.

use nidavellir_core::gpu_sweep::StabilityResult;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    iters: u32,
    n: u32,
    _pad: [u32; 2],
}

const KAT_SHADER: &str = r#"
struct Params { iters: u32, n: u32, _pad0: u32, _pad1: u32 };
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<uniform> p: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.n) { return; }
    var x = data[i];
    for (var k: u32 = 0u; k < p.iters; k = k + 1u) {
        // Numerical Recipes LCG — wrapping u32 mul/add, bit-exact vs CPU.
        x = x * 1664525u + 1013904223u;
    }
    data[i] = x;
}
"#;

/// CPU reference for the LCG kernel — must match the shader bit-for-bit.
fn lcg_reference(seed: u32, iters: u32) -> u32 {
    let mut x = seed;
    for _ in 0..iters {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
    }
    x
}

/// Result of a validation run.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub result: StabilityResult,
    pub elements: u32,
    pub iters: u32,
    pub mismatches: u32,
    pub elapsed_ms: u128,
    pub adapter: String,
}

/// Run the ALU known-answer test on the high-performance GPU.
///
/// `elements` = parallel lanes (≈ buffer size in u32), `iters` = LCG rounds per
/// lane (the ALU load knob). Returns a [`ValidationReport`]; a `SilentError`
/// means the GPU produced wrong results without crashing.
pub fn validate_kat(elements: u32, iters: u32) -> Result<ValidationReport, String> {
    let start = std::time::Instant::now();

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

    // A device-lost (driver TDR / hard instability) surfaces here.
    let crashed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let crashed = crashed.clone();
        device.on_uncaptured_error(Box::new(move |e| {
            eprintln!("wgpu device error: {e}");
            crashed.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
    }

    let input: Vec<u32> = (0..elements).collect();
    let byte_size = (elements as usize * std::mem::size_of::<u32>()) as u64;

    let storage = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("data"),
        contents: bytemuck::cast_slice(&input),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: byte_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("params"),
        contents: bytemuck::bytes_of(&Params { iters, n: elements, _pad: [0; 2] }),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("kat"),
        source: wgpu::ShaderSource::Wgsl(KAT_SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("kat"),
        layout: None,
        module: &module,
        entry_point: "main",
        compilation_options: wgpu::PipelineCompilationOptions::default(),
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("kat"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: storage.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: params.as_entire_binding() },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("kat"),
    });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kat"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        let groups = elements.div_ceil(64);
        cpass.dispatch_workgroups(groups, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&storage, 0, &staging, 0, byte_size);
    queue.submit(Some(encoder.finish()));

    // Read back.
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::Maintain::Wait);

    let mapped_ok = matches!(rx.recv(), Ok(Ok(())));
    if !mapped_ok || crashed.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(ValidationReport {
            result: StabilityResult::Crash,
            elements,
            iters,
            mismatches: 0,
            elapsed_ms: start.elapsed().as_millis(),
            adapter: adapter_name,
        });
    }

    let data = slice.get_mapped_range();
    let output: &[u32] = bytemuck::cast_slice(&data);

    let mut mismatches = 0u32;
    for (i, &got) in output.iter().enumerate() {
        if got != lcg_reference(i as u32, iters) {
            mismatches += 1;
        }
    }

    let result = if mismatches == 0 {
        StabilityResult::Stable
    } else {
        StabilityResult::SilentError
    };

    Ok(ValidationReport {
        result,
        elements,
        iters,
        mismatches,
        elapsed_ms: start.elapsed().as_millis(),
        adapter: adapter_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcg_reference_is_deterministic() {
        assert_eq!(lcg_reference(0, 1), 1013904223);
        assert_eq!(lcg_reference(0, 2), 1013904223u32.wrapping_mul(1664525).wrapping_add(1013904223));
        // Stable across calls.
        assert_eq!(lcg_reference(123, 1000), lcg_reference(123, 1000));
    }
}
