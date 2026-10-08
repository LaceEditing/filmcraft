//! The compositor on a device of its own, for exports ([`filmcraft_render::plan::PlanRenderer`]).
//!
//! Exports render frames on worker threads, away from the UI's device and its frame loop; a
//! headless device on the high-performance adapter (the discrete GPU where there is one)
//! composites them and the output is read back as RGBA8. Native only: creating the device and
//! reading back block.

use filmcraft_render::plan::{FramePlan, PlanRenderer};

use crate::GpuCompositor;

/// A [`GpuCompositor`] with its own device.
pub struct HeadlessRenderer {
    comp: GpuCompositor,
    name: String,
}

impl HeadlessRenderer {
    /// A renderer on the high-performance adapter; None without one (no GPU, no driver, or a
    /// device that cannot be created).
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
        )
        .ok()?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            // a software rasteriser is slower than FilmCraft's own CPU compositor
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("filmcraft-export"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()?;
        Some(Self { comp: GpuCompositor::new(&device, &queue), name: info.name })
    }
}

impl PlanRenderer for HeadlessRenderer {
    fn render_rgba8(&mut self, plan: &FramePlan) -> Option<Vec<u8>> {
        let (want_w, want_h) = match plan {
            FramePlan::Layers { width, height, .. } => (*width as u32, *height as u32),
            FramePlan::Image(img) => (img.w as u32, img.h as u32),
        };
        self.comp.composite(plan);
        let (w, h, px) = self.comp.read_output()?;
        ((w, h) == (want_w, want_h)).then_some(px)
    }
    fn name(&self) -> String {
        self.name.clone()
    }
}

fn factory() -> Option<Box<dyn PlanRenderer>> {
    match HeadlessRenderer::new() {
        Some(r) => {
            log::info!("export: compositing on the GPU ({})", r.name);
            Some(Box::new(r))
        }
        None => {
            log::info!("export: no GPU adapter for compositing; using the CPU");
            None
        }
    }
}

/// Make exports composite on the GPU ([`filmcraft_render::plan::register_plan_renderer`]).
pub fn register_plan_renderer() {
    filmcraft_render::plan::register_plan_renderer(factory);
}
