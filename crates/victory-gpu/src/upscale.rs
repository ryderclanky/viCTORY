//! The title's upscaler, stood in for.
//!
//! On the smaller console the title renders at 1280x720 and asks AMD's
//! upscaler (`amd_fidelityfx_upscaler_xbox.dll`) to make the 1920x1080
//! picture it presents. Those passes are console compute programs in a
//! container part the translator does not read, so none of them runs, the
//! output stays as it was cleared, and the window is black apart from what
//! was drawn into that output afterwards (a cursor). The title's loader
//! looks the upscaler's `ffxDispatch` up by name; [`hook`] is what that
//! lookup gets. An upscale dispatch is then one pass of ours that draws the
//! colour input over the output. Every other dispatch, and an upscale we
//! cannot draw, goes to the upscaler as before.
//!
//! `VICTORY_UPSCALE=title` leaves the lookup alone.
//!
//! The present also draws the title's second plane (its interface: menus,
//! HUD, text) over that picture. `VICTORY_UI_PLANE=0` leaves the plane out.

#[cfg(windows)]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

/// The description type of an upscale dispatch (`ffxDispatchDescUpscale`).
pub const UPSCALE_KIND: u64 = 0x0001_0001;

/// An upscale the title asked for: the list it is recording into, the colour
/// input, the output, the rendered rectangle and the output rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upscale {
    pub list: usize,
    pub colour: usize,
    pub output: usize,
    pub render: [u32; 2],
    pub size: [u32; 2],
}

/// What a dispatch description is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    Upscale(Upscale),
    /// Some other dispatch of the upscaler's. The value is its type word.
    Other(u64),
}

/// `VICTORY_UPSCALE=title`: the title's own upscaler runs.
pub fn leave_to_title(raw: Option<&str>) -> bool {
    matches!(raw.map(str::trim), Some("title"))
}

/// Whether a `GetProcAddress` of `name` in a module at `module_path` is the
/// upscaler dispatch we stand in for.
pub fn stands_in(name: &str, module_path: &str, upscale_env: Option<&str>) -> bool {
    !leave_to_title(upscale_env)
        && name == "ffxDispatch"
        && module_path
            .to_ascii_lowercase()
            .contains("fidelityfx_upscaler")
}

/// Header, then the list pointer, then seven resources of 48 bytes (the
/// pointer first). The colour input is the first resource; the output is
/// the last. Jitter and the motion scale follow (16 bytes), then the
/// rendered size and the upscale size.
const LIST_AT: usize = 16;
const COLOUR_AT: usize = 24;
const OUTPUT_AT: usize = 24 + 6 * 48;
const RENDER_AT: usize = OUTPUT_AT + 48 + 16;
const DESC_LEN: usize = RENDER_AT + 16;

/// Reads a dispatch description. `None` when `bytes` is too short to be the
/// kind it names.
pub fn read_dispatch(bytes: &[u8]) -> Option<Dispatch> {
    if bytes.len() < 8 {
        return None;
    }
    let kind = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    if kind != UPSCALE_KIND {
        return Some(Dispatch::Other(kind));
    }
    let bytes = bytes.get(..DESC_LEN)?;
    let pointer = |at: usize| {
        let low = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let high = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
        (u64::from(low) | u64::from(high) << 32) as usize
    };
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    Some(Dispatch::Upscale(Upscale {
        list: pointer(LIST_AT),
        colour: pointer(COLOUR_AT),
        output: pointer(OUTPUT_AT),
        render: [word(RENDER_AT), word(RENDER_AT + 4)],
        size: [word(RENDER_AT + 8), word(RENDER_AT + 12)],
    }))
}

/// Whether a rectangle is a size we will draw (each side 1 to 16384).
pub fn sane_extent(d: [u32; 2]) -> bool {
    (1..=16384).contains(&d[0]) && (1..=16384).contains(&d[1])
}

/// Pixel-shader constants for drawing `render` texels of a texture of
/// `source` texels over an output of `output` texels: `(scale.xy, limit.xy)`.
/// A pixel at output position `p` samples `min(p * scale, limit)`.
pub fn sample_constants(render: [u32; 2], output: [u32; 2], source: [f32; 2]) -> [f32; 4] {
    let (w, h) = (source[0].max(1.0), source[1].max(1.0));
    let (out_w, out_h) = (output[0].max(1) as f32, output[1].max(1) as f32);
    [
        render[0] as f32 / (out_w * w),
        render[1] as f32 / (out_h * h),
        (render[0] as f32 - 0.5) / w,
        (render[1] as f32 - 0.5) / h,
    ]
}

/// The upscaler's own `ffxDispatch`, kept so a dispatch we do not draw can
/// still reach it.
#[cfg(windows)]
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// What the title's lookup of `ffxDispatch` gets. `original` is the
/// upscaler's own export.
pub fn hook(original: usize) -> usize {
    #[cfg(windows)]
    {
        ORIGINAL.store(original, Relaxed);
        tracing::info!(
            original = format_args!("{original:#x}"),
            "the title's upscaler: its dispatch goes through ours (VICTORY_UPSCALE=title: left alone)"
        );
        dispatch as *const () as usize
    }
    #[cfg(not(windows))]
    {
        original
    }
}

/// Dispatches seen, upscales we drew, upscales handed on.
#[cfg(windows)]
static CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

#[cfg(windows)]
fn hand_on(context: *mut std::ffi::c_void, description: *const u8) -> u32 {
    match ORIGINAL.load(Relaxed) {
        // The interface's "no backend".
        0 => 1,
        // SAFETY: the address the upscaler exports its dispatch at.
        address => unsafe {
            let f = std::mem::transmute::<
                usize,
                extern "system" fn(*mut std::ffi::c_void, *const u8) -> u32,
            >(address);
            f(context, description)
        },
    }
}

/// `ffxDispatch(context, description)`.
#[cfg(windows)]
extern "system" fn dispatch(context: *mut std::ffi::c_void, description: *const u8) -> u32 {
    if description.is_null() {
        return hand_on(context, description);
    }
    // SAFETY: the title passes a description. A short one is refused below
    // after the kind word; an upscale is `DESC_LEN` bytes.
    let kind = unsafe { description.cast::<u64>().read_unaligned() };
    let call = CALLS[0].fetch_add(1, Relaxed);
    if kind != UPSCALE_KIND {
        if call < 8 {
            tracing::info!(
                kind = format_args!("{kind:#x}"),
                "the title's upscaler: a dispatch that is no upscale, handed on"
            );
        }
        return hand_on(context, description);
    }
    // SAFETY: an upscale description is `DESC_LEN` bytes. A shorter one
    // makes `read_dispatch` refuse it and the call is handed on.
    let bytes = unsafe { std::slice::from_raw_parts(description, DESC_LEN) };
    let Some(Dispatch::Upscale(up)) = read_dispatch(bytes) else {
        return hand_on(context, description);
    };
    if call < 2 {
        tracing::info!(
            list = format_args!("{:#x}", up.list),
            colour = format_args!("{:#x}", up.colour),
            output = format_args!("{:#x}", up.output),
            render = ?up.render,
            size = ?up.size,
            "the title's upscaler: an upscale dispatch"
        );
    }
    let drawn = up.list != 0
        && up.colour != 0
        && up.output != 0
        && sane_extent(up.render)
        && sane_extent(up.size)
        && crate::list::upscale(
            up.list as crate::object::This,
            up.colour as crate::object::This,
            up.output as crate::object::This,
            up.render,
            up.size,
        );
    if drawn {
        CALLS[1].fetch_add(1, Relaxed);
        // The interface's "ok".
        return 0;
    }
    if CALLS[2].fetch_add(1, Relaxed) < 4 {
        tracing::warn!(
            ?up.render,
            ?up.size,
            "the title's upscaler: an upscale we could not draw ourselves, handed on"
        );
    }
    hand_on(context, description)
}

/// Whether the interface plane is drawn (`VICTORY_UI_PLANE`).
pub(crate) fn ui_plane_on() -> bool {
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        crate::laptop::ui_plane_mode(std::env::var("VICTORY_UI_PLANE").ok().as_deref())
    }) != 0
}

/// The second present plane, when it is an interface drawn over `picture`.
/// `second` is `(resource count, first resource)`.
pub(crate) fn keep_interface(
    ui_on: bool,
    picture: usize,
    second: Option<(u32, usize)>,
) -> Option<usize> {
    if !ui_on {
        return None;
    }
    let (count, resource) = second?;
    (count > 0 && resource != 0 && resource != picture).then_some(resource)
}

#[cfg(windows)]
mod pass {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows::Win32::Graphics::Direct3D12::*;
    use windows::Win32::Graphics::Dxgi::Common::*;

    use crate::host::{self, HostList};

    type Pipe = (ID3D12RootSignature, ID3D12PipelineState);

    struct Pass {
        views: ID3D12DescriptorHeap,
        cpu: usize,
        gpu: u64,
        step: u32,
        next: AtomicU32,
        upscale: Mutex<Vec<(i32, Option<Pipe>)>>,
        overlay: Mutex<Vec<(i32, Option<Pipe>)>>,
    }

    // SAFETY: D3D12 objects are free-threaded.
    unsafe impl Send for Pass {}
    // SAFETY: as above.
    unsafe impl Sync for Pass {}

    const VIEWS: u32 = 64;

    const CORNERS: &str = r#"
float4 corner(uint i : SV_VertexID) : SV_Position {
    return float4(float(i & 1) * 4.0 - 1.0, float(i >> 1) * 4.0 - 1.0, 0.0, 1.0);
}
"#;

    /// The rendered part of the colour input, over the output, linear.
    const UPSCALE: &str = r#"
#define RS "DescriptorTable(SRV(t0), visibility = SHADER_VISIBILITY_PIXEL), RootConstants(num32BitConstants = 4, b0, visibility = SHADER_VISIBILITY_PIXEL), StaticSampler(s0, filter = FILTER_MIN_MAG_LINEAR_MIP_POINT, addressU = TEXTURE_ADDRESS_CLAMP, addressV = TEXTURE_ADDRESS_CLAMP, addressW = TEXTURE_ADDRESS_CLAMP, visibility = SHADER_VISIBILITY_PIXEL)"
Texture2D<float4> colour : register(t0);
SamplerState smooth : register(s0);
cbuffer sizes : register(b0) { float2 scale; float2 limit; };
[RootSignature(RS)]
float4 upscale(float4 p : SV_Position) : SV_Target {
    return float4(colour.SampleLevel(smooth, min(p.xy * scale, limit), 0).rgb, 1.0);
}
"#;

    /// The interface plane, stretched over the picture. Premultiplied by
    /// default; straight alpha is the pipeline, not this shader.
    const OVERLAY: &str = r#"
#define RS "DescriptorTable(SRV(t0), visibility = SHADER_VISIBILITY_PIXEL), RootConstants(num32BitConstants = 4, b0, visibility = SHADER_VISIBILITY_PIXEL), StaticSampler(s0, filter = FILTER_MIN_MAG_LINEAR_MIP_POINT, addressU = TEXTURE_ADDRESS_CLAMP, addressV = TEXTURE_ADDRESS_CLAMP, addressW = TEXTURE_ADDRESS_CLAMP, visibility = SHADER_VISIBILITY_PIXEL)"
Texture2D<float4> plane : register(t0);
SamplerState smooth : register(s0);
cbuffer sizes : register(b0) { float2 scale; float2 unused; };
[RootSignature(RS)]
float4 overlay(float4 p : SV_Position) : SV_Target {
    return plane.SampleLevel(smooth, p.xy * scale, 0);
}
"#;

    fn pass() -> Option<&'static Pass> {
        static PASS: std::sync::OnceLock<Option<Pass>> = std::sync::OnceLock::new();
        PASS.get_or_init(|| {
            let host = host::host()?;
            let kind = D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV;
            // SAFETY: plain heap creation and queries.
            unsafe {
                let views: ID3D12DescriptorHeap = host
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: kind,
                        NumDescriptors: VIEWS,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    })
                    .ok()?;
                Some(Pass {
                    cpu: views.GetCPUDescriptorHandleForHeapStart().ptr,
                    gpu: views.GetGPUDescriptorHandleForHeapStart().ptr,
                    step: host.device.GetDescriptorHandleIncrementSize(kind),
                    views,
                    next: AtomicU32::new(0),
                    upscale: Mutex::default(),
                    overlay: Mutex::default(),
                })
            }
        })
        .as_ref()
    }

    /// The plain format of a typeless target family.
    fn plain(f: DXGI_FORMAT) -> DXGI_FORMAT {
        DXGI_FORMAT(match f.0 {
            9 => 10,
            23 => 24,
            27 => 28,
            other => other,
        })
    }

    fn pipeline(
        made: &Mutex<Vec<(i32, Option<Pipe>)>>,
        format: DXGI_FORMAT,
        source: &'static str,
        entry: &str,
        blend: bool,
    ) -> Option<Pipe> {
        let mut made = made.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, pipe)) = made.iter().find(|m| m.0 == format.0) {
            return pipe.clone();
        }
        let make = || -> windows::core::Result<Pipe> {
            use windows::core::s;
            let device = &host::host().ok_or_else(windows::core::Error::empty)?.device;
            let vs = host::compile(CORNERS, s!("corner"), s!("vs_5_1"))?;
            let ps_entry = if blend { s!("overlay") } else { s!("upscale") };
            let ps = host::compile(source, ps_entry, s!("ps_5_1"))?;
            let code = |bytes: &[u8]| D3D12_SHADER_BYTECODE {
                pShaderBytecode: bytes.as_ptr().cast(),
                BytecodeLength: bytes.len(),
            };
            let mut blend_desc = D3D12_BLEND_DESC::default();
            let rt = &mut blend_desc.RenderTarget[0];
            rt.RenderTargetWriteMask = 15;
            if blend {
                rt.BlendEnable = true.into();
                // Straight alpha when asked; otherwise the console's
                // premultiplied interface plane.
                rt.SrcBlend = if ui_straight() {
                    D3D12_BLEND_SRC_ALPHA
                } else {
                    D3D12_BLEND_ONE
                };
                rt.DestBlend = D3D12_BLEND_INV_SRC_ALPHA;
                rt.BlendOp = D3D12_BLEND_OP_ADD;
                rt.SrcBlendAlpha = D3D12_BLEND_ZERO;
                rt.DestBlendAlpha = D3D12_BLEND_ONE;
                rt.BlendOpAlpha = D3D12_BLEND_OP_ADD;
            }
            let mut formats = [DXGI_FORMAT(0); 8];
            formats[0] = format;
            // SAFETY: the root signature is inside the pixel shader; the
            // description points at live data and never releases its pointer.
            unsafe {
                let root: ID3D12RootSignature = device.CreateRootSignature(0, &ps[..])?;
                let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                    pRootSignature: std::mem::transmute_copy(&root),
                    VS: code(&vs),
                    PS: code(&ps),
                    BlendState: blend_desc,
                    SampleMask: u32::MAX,
                    RasterizerState: D3D12_RASTERIZER_DESC {
                        FillMode: D3D12_FILL_MODE_SOLID,
                        CullMode: D3D12_CULL_MODE_NONE,
                        DepthClipEnable: true.into(),
                        ..Default::default()
                    },
                    PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                    NumRenderTargets: 1,
                    RTVFormats: formats,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    ..Default::default()
                };
                let pipeline = device.CreateGraphicsPipelineState(&desc)?;
                Ok((root, pipeline))
            }
        };
        let pipe = match make() {
            Ok(pipe) => Some(pipe),
            Err(e) => {
                tracing::warn!(error = %e, format = format.0, entry, "no pipeline for a present pass of ours");
                None
            }
        };
        made.push((format.0, pipe.clone()));
        pipe
    }

    fn ui_straight() -> bool {
        static STRAIGHT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *STRAIGHT.get_or_init(|| {
            crate::laptop::ui_plane_mode(std::env::var("VICTORY_UI_PLANE").ok().as_deref()) == 2
        })
    }

    struct Draw<'a> {
        list: &'a ID3D12GraphicsCommandList,
        colour: &'a ID3D12Resource,
        view: &'a D3D12_SHADER_RESOURCE_VIEW_DESC,
        target: D3D12_CPU_DESCRIPTOR_HANDLE,
        root: &'a ID3D12RootSignature,
        pipeline: &'a ID3D12PipelineState,
        constants: &'a [f32; 4],
        width: u32,
        height: u32,
    }

    fn draw(pass_draw: Draw<'_>) {
        let Some(host) = host::host() else {
            return;
        };
        let Some(pass) = pass() else {
            return;
        };
        let slot = pass.next.fetch_add(1, Ordering::Relaxed) % VIEWS;
        let cpu = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: pass.cpu + (slot * pass.step) as usize,
        };
        let gpu = D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: pass.gpu + u64::from(slot * pass.step),
        };
        // SAFETY: an open list; a slot of our heap; live resources and
        // objects (the pass's own live as long as the process); four constants.
        unsafe {
            host.device
                .CreateShaderResourceView(pass_draw.colour, Some(pass_draw.view), cpu);
            pass_draw
                .list
                .SetDescriptorHeaps(&[Some(pass.views.clone())]);
            pass_draw.list.SetGraphicsRootSignature(pass_draw.root);
            pass_draw.list.SetPipelineState(pass_draw.pipeline);
            pass_draw.list.SetGraphicsRootDescriptorTable(0, gpu);
            pass_draw.list.SetGraphicsRoot32BitConstants(
                1,
                4,
                pass_draw.constants.as_ptr().cast(),
                0,
            );
            pass_draw
                .list
                .OMSetRenderTargets(1, Some(&pass_draw.target), false, None);
            pass_draw.list.RSSetViewports(&[D3D12_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: pass_draw.width as f32,
                Height: pass_draw.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]);
            pass_draw
                .list
                .RSSetScissorRects(&[windows::Win32::Foundation::RECT {
                    left: 0,
                    top: 0,
                    right: pass_draw.width as i32,
                    bottom: pass_draw.height as i32,
                }]);
            pass_draw.list.IASetPrimitiveTopology(
                windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY(4),
            );
            pass_draw.list.DrawInstanced(3, 1, 0, 0);
        }
    }

    fn view(format: DXGI_FORMAT) -> D3D12_SHADER_RESOURCE_VIEW_DESC {
        D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: format,
            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_SRV {
                    MostDetailedMip: 0,
                    MipLevels: 1,
                    ..Default::default()
                },
            },
        }
    }

    /// Draws the first `render` texels of `colour` over the first `size`
    /// texels of `output`. `output` is a render target. False if nothing
    /// was recorded. The caller sets the title's state again afterwards.
    pub(crate) fn record_upscale(
        list: &HostList,
        colour: &ID3D12Resource,
        output: &ID3D12Resource,
        render: [u32; 2],
        size: [u32; 2],
    ) -> bool {
        let Some(host) = host::host() else {
            return false;
        };
        let Some(pass) = pass() else {
            return false;
        };
        // SAFETY: live resources.
        let (from, to) = unsafe { (colour.GetDesc(), output.GetDesc()) };
        if to.Flags.0 & D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET.0 == 0 {
            tracing::warn!(
                format = to.Format.0,
                flags = to.Flags.0,
                "the upscaler's output is no render target here"
            );
            return false;
        }
        let Some((root, pipeline)) =
            pipeline(&pass.upscale, plain(to.Format), UPSCALE, "upscale", false)
        else {
            return false;
        };
        let (width, height) = (size[0].min(to.Width as u32), size[1].min(to.Height));
        let constants = super::sample_constants(
            render,
            [width, height],
            [from.Width as f32, from.Height as f32],
        );
        let target = D3D12_RENDER_TARGET_VIEW_DESC {
            Format: plain(to.Format),
            ViewDimension: D3D12_RTV_DIMENSION_TEXTURE2D,
            Anonymous: D3D12_RENDER_TARGET_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_RTV {
                    MipSlice: 0,
                    PlaneSlice: 0,
                },
            },
        };
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            tracing::info!(
                from = format_args!(
                    "{}x{} of {}x{} f{}",
                    render[0], render[1], from.Width, from.Height, from.Format.0
                ),
                to = format_args!(
                    "{width}x{height} of {}x{} f{}",
                    to.Width, to.Height, to.Format.0
                ),
                "the title's upscaler stood in for: its colour input drawn over its output"
            );
        });
        let source = view(plain(from.Format));
        draw(Draw {
            list: &list.list,
            colour,
            view: &source,
            target: host.rtv(Some(output), Some(&target)),
            root: &root,
            pipeline: &pipeline,
            constants: &constants,
            width,
            height,
        });
        true
    }

    /// Draws `plane` (readable by pixel shaders) over `back`, a render
    /// target of `format`, `width` by `height`. False if nothing was recorded.
    pub(crate) fn record_overlay(
        list: &ID3D12GraphicsCommandList,
        plane: &ID3D12Resource,
        back: &ID3D12Resource,
        format: DXGI_FORMAT,
        width: u32,
        height: u32,
    ) -> bool {
        let Some(host) = host::host() else {
            return false;
        };
        let Some(pass) = pass() else {
            return false;
        };
        let Some((root, pipeline)) = pipeline(&pass.overlay, format, OVERLAY, "overlay", true)
        else {
            return false;
        };
        // SAFETY: a live resource.
        let from = unsafe { plane.GetDesc() };
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            tracing::info!(
                plane = format_args!("{}x{} f{}", from.Width, from.Height, from.Format.0),
                window = format_args!("{width}x{height} f{}", format.0),
                "the title's interface plane drawn over its picture (VICTORY_UI_PLANE=0: left out)"
            );
        });
        let constants = [
            1.0 / width.max(1) as f32,
            1.0 / height.max(1) as f32,
            0.0,
            0.0,
        ];
        let source = view(plain(from.Format));
        draw(Draw {
            list,
            colour: plane,
            view: &source,
            target: host.rtv(Some(back), None),
            root: &root,
            pipeline: &pipeline,
            constants: &constants,
            width,
            height,
        });
        true
    }
}

#[cfg(windows)]
pub(crate) use pass::{record_overlay, record_upscale};

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u32(buf: &mut [u8], at: usize, value: u32) {
        buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_ptr(buf: &mut [u8], at: usize, value: usize) {
        put_u32(buf, at, value as u32);
        put_u32(buf, at + 4, (value as u64 >> 32) as u32);
    }

    /// The Series S frame from run 304: render 1280x720, present 1920x1080.
    fn series_s_upscale() -> Vec<u8> {
        let mut bytes = vec![0u8; DESC_LEN];
        put_u32(&mut bytes, 0, UPSCALE_KIND as u32);
        put_u32(&mut bytes, 4, (UPSCALE_KIND >> 32) as u32);
        put_ptr(&mut bytes, LIST_AT, 0x1_0000);
        put_ptr(&mut bytes, COLOUR_AT, 0x2_0000);
        put_ptr(&mut bytes, OUTPUT_AT, 0x3_0000);
        put_u32(&mut bytes, RENDER_AT, 1280);
        put_u32(&mut bytes, RENDER_AT + 4, 720);
        put_u32(&mut bytes, RENDER_AT + 8, 1920);
        put_u32(&mut bytes, RENDER_AT + 12, 1080);
        bytes
    }

    #[test]
    fn an_upscale_names_its_list_colour_and_output() {
        let up = match read_dispatch(&series_s_upscale()) {
            Some(Dispatch::Upscale(up)) => up,
            other => panic!("expected an upscale, got {other:?}"),
        };
        assert_eq!(
            up,
            Upscale {
                list: 0x1_0000,
                colour: 0x2_0000,
                output: 0x3_0000,
                render: [1280, 720],
                size: [1920, 1080],
            }
        );
        assert!(sane_extent(up.render) && sane_extent(up.size));
        assert!(!sane_extent([0, 720]));
        assert!(!sane_extent([1, 20000]));
    }

    #[test]
    fn other_dispatches_and_short_descriptions_are_not_upscales() {
        assert_eq!(read_dispatch(&[]), None);
        assert_eq!(read_dispatch(&[0; 7]), None);
        let mut other = [0u8; 8];
        other[0] = 2;
        assert_eq!(read_dispatch(&other), Some(Dispatch::Other(2)));
        let mut short = series_s_upscale();
        short.truncate(32);
        assert_eq!(read_dispatch(&short), None);
    }

    #[test]
    fn only_the_upscaler_dispatch_is_stood_in_for() {
        let dll = r"D:\title\amd_fidelityfx_upscaler_xbox.dll";
        assert!(stands_in("ffxDispatch", dll, None));
        assert!(stands_in("ffxDispatch", dll, Some("")));
        assert!(!stands_in("ffxDispatch", dll, Some("title")));
        assert!(!stands_in("ffxQuery", dll, None));
        assert!(!stands_in(
            "ffxDispatch",
            r"D:\title\amd_fidelityfx_loader.dll",
            None
        ));
    }

    #[test]
    fn a_series_s_frame_samples_the_rendered_rect_and_not_past_it() {
        // The colour target is the rendered size.
        let c = sample_constants([1280, 720], [1920, 1080], [1280.0, 720.0]);
        let u = (1920.0 * c[0]).min(c[2]);
        let v = (1080.0 * c[1]).min(c[3]);
        assert!((u - (1279.5 / 1280.0)).abs() < 1e-5, "{u}");
        assert!((v - (719.5 / 720.0)).abs() < 1e-5, "{v}");
        // A larger target still stops at the rendered rectangle.
        let c = sample_constants([1280, 720], [1920, 1080], [1920.0, 1080.0]);
        let u = (1920.0 * c[0]).min(c[2]);
        let v = (1080.0 * c[1]).min(c[3]);
        assert!((u - (1279.5 / 1920.0)).abs() < 1e-5, "{u}");
        assert!((v - (719.5 / 1080.0)).abs() < 1e-5, "{v}");
    }

    #[test]
    fn the_interface_plane_is_a_second_resource() {
        assert_eq!(keep_interface(true, 1, Some((1, 2))), Some(2));
        assert_eq!(keep_interface(false, 1, Some((1, 2))), None);
        assert_eq!(keep_interface(true, 1, None), None);
        assert_eq!(keep_interface(true, 1, Some((0, 2))), None);
        assert_eq!(keep_interface(true, 1, Some((1, 0))), None);
        assert_eq!(keep_interface(true, 1, Some((1, 1))), None);
    }
}
