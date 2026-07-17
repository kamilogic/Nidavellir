use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::{Dx11AdapterIdentity, Dx11Golden, Dx11QualificationResult};
use nidavellir_core::gpu_sweep::StabilityResult;
use windows::core::PCSTR;
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1};

const NVIDIA_VENDOR_ID: u32 = 0x10de;
const TARGET_WIDTH: u32 = 768;
const TARGET_HEIGHT: u32 = 768;
const GOLDEN_MIN_CHECKS: u32 = 3;
const CHECK_INTERVAL_FRAMES: u64 = 24;
const GPU_COMPLETION_TIMEOUT: Duration = Duration::from_millis(750);

const SHADER_SOURCE: &[u8] = br#"
struct VsOut { float4 position : SV_Position; float2 uv : TEXCOORD0; };

VsOut vs_main(uint id : SV_VertexID) {
    float2 p = float2((id << 1) & 2, id & 2);
    VsOut o;
    o.position = float4(p * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    o.uv = p;
    return o;
}

float4 ps_main(VsOut input) : SV_Target {
    float3 v = float3(input.uv, input.uv.x * input.uv.y) + 0.03125;
    [unroll] for (uint i = 0; i < 48; ++i) {
        v = frac(v.yzx * float3(1.6180339, 1.4142135, 1.7320508)
            + v.zxy * 0.375 + float3(0.013, 0.017, 0.019));
        v = mad(v, 0.875, v.zxy * 0.125);
    }
    return float4(v, 1.0);
}
"#;

pub struct Dx11Qualifier {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    render_target: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    render_target_view: ID3D11RenderTargetView,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    completion_query: ID3D11Query,
    adapter: Dx11AdapterIdentity,
}

impl Dx11Qualifier {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let (adapter, adapter_identity) = select_nvidia_adapter()?;
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                None,
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice failed: {e}"))?;
            let device = device.ok_or_else(|| "D3D11 device was not returned".to_string())?;
            let context =
                context.ok_or_else(|| "D3D11 immediate context was not returned".to_string())?;

            let vertex_blob = compile_shader(b"vs_main\0", b"vs_5_0\0")?;
            let pixel_blob = compile_shader(b"ps_main\0", b"ps_5_0\0")?;
            let vertex_bytes = std::slice::from_raw_parts(
                vertex_blob.GetBufferPointer().cast::<u8>(),
                vertex_blob.GetBufferSize(),
            );
            let pixel_bytes = std::slice::from_raw_parts(
                pixel_blob.GetBufferPointer().cast::<u8>(),
                pixel_blob.GetBufferSize(),
            );
            let mut vertex_shader = None;
            device
                .CreateVertexShader(vertex_bytes, None, Some(&mut vertex_shader))
                .map_err(|e| format!("CreateVertexShader failed: {e}"))?;
            let mut pixel_shader = None;
            device
                .CreatePixelShader(pixel_bytes, None, Some(&mut pixel_shader))
                .map_err(|e| format!("CreatePixelShader failed: {e}"))?;

            let target_desc = D3D11_TEXTURE2D_DESC {
                Width: TARGET_WIDTH,
                Height: TARGET_HEIGHT,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut render_target = None;
            device
                .CreateTexture2D(&target_desc, None, Some(&mut render_target))
                .map_err(|e| format!("CreateTexture2D(render target) failed: {e}"))?;
            let render_target =
                render_target.ok_or_else(|| "D3D11 render target was not returned".to_string())?;
            let mut render_target_view = None;
            device
                .CreateRenderTargetView(&render_target, None, Some(&mut render_target_view))
                .map_err(|e| format!("CreateRenderTargetView failed: {e}"))?;

            let staging_desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..target_desc
            };
            let mut staging = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .map_err(|e| format!("CreateTexture2D(staging) failed: {e}"))?;

            let query_desc = D3D11_QUERY_DESC {
                Query: D3D11_QUERY_EVENT,
                MiscFlags: 0,
            };
            let mut completion_query = None;
            device
                .CreateQuery(&query_desc, Some(&mut completion_query))
                .map_err(|e| format!("CreateQuery failed: {e}"))?;

            Ok(Self {
                device,
                context,
                render_target,
                staging: staging
                    .ok_or_else(|| "D3D11 staging texture was not returned".to_string())?,
                render_target_view: render_target_view
                    .ok_or_else(|| "D3D11 render-target view was not returned".to_string())?,
                vertex_shader: vertex_shader
                    .ok_or_else(|| "D3D11 vertex shader was not returned".to_string())?,
                pixel_shader: pixel_shader
                    .ok_or_else(|| "D3D11 pixel shader was not returned".to_string())?,
                completion_query: completion_query
                    .ok_or_else(|| "D3D11 completion query was not returned".to_string())?,
                adapter: adapter_identity,
            })
        }
    }

    pub fn adapter_identity(&self) -> Dx11AdapterIdentity {
        self.adapter.clone()
    }

    pub fn capture_golden(&self, sample_ms: u64) -> Result<Dx11Golden, String> {
        let started = Instant::now();
        let mut checks = Vec::new();
        let mut frames = 0u64;
        while started.elapsed() < Duration::from_millis(sample_ms)
            || checks.len() < GOLDEN_MIN_CHECKS as usize
        {
            self.draw_frame();
            self.wait_for_gpu_completion()?;
            frames = frames.saturating_add(1);
            if frames.is_multiple_of(CHECK_INTERVAL_FRAMES) {
                checks.push(self.readback_checksum()?);
            }
        }
        let Some(&checksum) = checks.first() else {
            return Err("DX11 golden captured no checksum".into());
        };
        if checks.iter().any(|candidate| *candidate != checksum) {
            return Err("DX11 stock golden was not deterministic".into());
        }
        let elapsed_us = started.elapsed().as_micros().max(1) as u64;
        Ok(Dx11Golden {
            checksum,
            adapter_luid: self.adapter.adapter_luid,
            frame_reference_us: (elapsed_us / frames.max(1)).clamp(1, u64::from(u32::MAX)) as u32,
        })
    }

    pub fn run_with_golden(
        &self,
        duration_ms: u64,
        golden: Dx11Golden,
        cancel: Option<&AtomicBool>,
    ) -> Dx11QualificationResult {
        let started = Instant::now();
        if golden.adapter_luid != self.adapter.adapter_luid {
            return result(
                StabilityResult::Stable,
                0,
                0,
                started,
                false,
                Some("stock/candidate adapter LUID mismatch".into()),
            );
        }
        let mut frames = 0u64;
        let mut checks = 0u32;
        while started.elapsed() < Duration::from_millis(duration_ms) {
            if cancel.is_some_and(|token| token.load(Ordering::SeqCst)) {
                break;
            }
            self.draw_frame();
            if let Err(error) = self.wait_for_gpu_completion() {
                let timed_out = error.contains("completion timeout");
                return result(
                    if timed_out {
                        StabilityResult::Unstable
                    } else {
                        StabilityResult::Crash
                    },
                    frames,
                    checks,
                    started,
                    timed_out,
                    None,
                );
            }
            frames = frames.saturating_add(1);
            if frames.is_multiple_of(CHECK_INTERVAL_FRAMES) {
                match self.readback_checksum() {
                    Ok(checksum) if checksum == golden.checksum => {
                        checks = checks.saturating_add(1)
                    }
                    Ok(_) => {
                        return result(
                            StabilityResult::SilentError,
                            frames,
                            checks,
                            started,
                            false,
                            None,
                        )
                    }
                    Err(error) => {
                        let timed_out = error.contains("completion timeout");
                        return result(
                            if timed_out {
                                StabilityResult::Unstable
                            } else {
                                StabilityResult::Crash
                            },
                            frames,
                            checks,
                            started,
                            timed_out,
                            None,
                        );
                    }
                }
            }
        }
        let verdict = if checks == 0 {
            StabilityResult::Crash
        } else {
            StabilityResult::Stable
        };
        result(verdict, frames, checks, started, false, None)
    }

    fn draw_frame(&self) {
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(self.render_target_view.clone())]), None);
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: TARGET_WIDTH as f32,
                Height: TARGET_HEIGHT as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context.DrawInstanced(3, 4, 0, 0);
        }
    }

    fn readback_checksum(&self) -> Result<u32, String> {
        unsafe {
            self.context
                .CopyResource(&self.staging, &self.render_target);
            self.wait_for_gpu_completion()?;

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| format!("DX11 staging Map failed: {e}"))?;
            let mut hash = 0x811c9dc5u32;
            for row in 0..TARGET_HEIGHT as usize {
                let bytes = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(row * mapped.RowPitch as usize),
                    TARGET_WIDTH as usize * 4,
                );
                for byte in bytes {
                    hash = hash.wrapping_mul(0x01000193) ^ u32::from(*byte);
                }
            }
            self.context.Unmap(&self.staging, 0);
            Ok(hash)
        }
    }

    fn wait_for_gpu_completion(&self) -> Result<(), String> {
        unsafe {
            self.context.End(&self.completion_query);
            self.context.Flush();
            let wait_started = Instant::now();
            loop {
                let mut completed = BOOL(0);
                match self.context.GetData(
                    &self.completion_query,
                    Some((&mut completed as *mut BOOL).cast::<c_void>()),
                    std::mem::size_of::<BOOL>() as u32,
                    D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32,
                ) {
                    Ok(()) if completed.as_bool() => return Ok(()),
                    Ok(()) => {}
                    Err(e) => return Err(format!("DX11 completion query failed: {e}")),
                }
                if wait_started.elapsed() >= GPU_COMPLETION_TIMEOUT {
                    let reason = self.device.GetDeviceRemovedReason().err();
                    return Err(format!(
                        "DX11 completion timeout; device_removed={reason:?}"
                    ));
                }
                std::thread::yield_now();
            }
        }
    }
}

fn result(
    verdict: StabilityResult,
    frames: u64,
    checks: u32,
    started: Instant,
    timed_out: bool,
    inconclusive_reason: Option<String>,
) -> Dx11QualificationResult {
    let elapsed = started.elapsed();
    Dx11QualificationResult {
        result: verdict,
        frames,
        checks,
        fps: frames as f64 / elapsed.as_secs_f64().max(f64::EPSILON),
        elapsed_ms: elapsed.as_millis().try_into().unwrap_or(u64::MAX),
        timed_out,
        inconclusive_reason,
    }
}

unsafe fn select_nvidia_adapter() -> Result<(IDXGIAdapter1, Dx11AdapterIdentity), String> {
    let factory: IDXGIFactory1 =
        CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1 failed: {e}"))?;
    let mut candidates = Vec::new();
    for index in 0..32 {
        let Ok(adapter) = factory.EnumAdapters1(index) else {
            break;
        };
        let desc = adapter
            .GetDesc1()
            .map_err(|e| format!("IDXGIAdapter1::GetDesc1 failed: {e}"))?;
        if desc.VendorId == NVIDIA_VENDOR_ID {
            candidates.push((desc.DedicatedVideoMemory, adapter, desc));
        }
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    let (_, adapter, desc) = candidates
        .into_iter()
        .next()
        .ok_or_else(|| "no NVIDIA DX11 adapter found".to_string())?;
    let name_end = desc
        .Description
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(desc.Description.len());
    let luid = (i64::from(desc.AdapterLuid.HighPart) << 32) | i64::from(desc.AdapterLuid.LowPart);
    Ok((
        adapter,
        Dx11AdapterIdentity {
            name: String::from_utf16_lossy(&desc.Description[..name_end]),
            vendor_id: desc.VendorId,
            device_id: desc.DeviceId,
            adapter_luid: luid,
        },
    ))
}

unsafe fn compile_shader(entry: &[u8], target: &[u8]) -> Result<ID3DBlob, String> {
    let mut code = None;
    let mut errors = None;
    let compiled = D3DCompile(
        SHADER_SOURCE.as_ptr().cast::<c_void>(),
        SHADER_SOURCE.len(),
        PCSTR::null(),
        None,
        None,
        PCSTR(entry.as_ptr()),
        PCSTR(target.as_ptr()),
        0,
        0,
        &mut code,
        Some(&mut errors),
    );
    if let Err(error) = compiled {
        let details = errors.map(|blob: ID3DBlob| {
            let bytes = std::slice::from_raw_parts(
                blob.GetBufferPointer().cast::<u8>(),
                blob.GetBufferSize(),
            );
            String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .to_string()
        });
        return Err(format!(
            "D3DCompile failed: {error}; {}",
            details.unwrap_or_default()
        ));
    }
    code.ok_or_else(|| "D3DCompile returned no shader bytecode".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "stock-GPU integration smoke; run explicitly on Windows hardware"]
    fn stock_golden_and_candidate_readback_are_stable() {
        let stock = Dx11Qualifier::new().expect("native DX11 stock context");
        assert_eq!(stock.adapter_identity().vendor_id, NVIDIA_VENDOR_ID);
        let golden = stock
            .capture_golden(1_000)
            .expect("deterministic stock golden");
        drop(stock);
        let candidate = Dx11Qualifier::new().expect("fresh native DX11 candidate context");
        let run = candidate.run_with_golden(1_000, golden, None);
        assert_eq!(run.result, StabilityResult::Stable);
        assert!(run.checks > 0);
        assert!(!run.timed_out);
    }
}
