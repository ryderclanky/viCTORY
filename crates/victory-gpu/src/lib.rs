//! D3D12.X on host D3D12.
//!
//! D3D12.X records commands from code inlined into the title, which writes
//! packets into the command list while its put pointer is below a limit and
//! calls the list's method otherwise. Our lists keep the limits at zero, so
//! every command arrives here as a call, to be run on host D3D12. The
//! title's GPU addresses are CPU addresses (console memory is unified).
//!
//! [`device`] is the device object titles create, [`dxgi`] its adapter side,
//! [`resource`] heaps and resources, [`fence`] fences, [`queue`] command
//! queues, [`list`] command allocators and lists, [`descriptor`]
//! descriptor heaps, [`root`] root signatures, [`query`] query heaps,
//! [`signature`] command signatures, [`pipeline`] pipeline states,
//! [`frame`] frame pacing, [`object`] what all console graphics objects
//! share, [`console`] which console model the title sees. Planned:
//! `present` (swap chain).

#[cfg(windows)]
mod ambient;
#[cfg(all(windows, feature = "diagnostics"))]
mod backreads;
#[cfg(all(windows, feature = "diagnostics"))]
mod capture;
#[cfg(windows)]
mod cardmeter;
pub mod console;
#[cfg(all(windows, feature = "diagnostics"))]
mod copystat;
pub mod descriptor;
pub mod device;
#[cfg(windows)]
mod draw;
#[cfg(windows)]
mod drvwatch;
pub mod dxgi;
pub mod fence;
#[cfg(windows)]
mod fpsbox;
pub mod frame;
#[cfg(windows)]
pub mod framelog;
#[cfg_attr(not(windows), allow(dead_code))]
mod heap_pass;
#[cfg(windows)]
mod heap_worker;
#[cfg(windows)]
mod host;
#[cfg(windows)]
mod indirect;
#[cfg(windows)]
mod inplace;
pub mod list;
#[cfg(windows)]
mod memory;
pub mod object;
pub mod pipeline;
#[cfg(windows)]
mod plaincopy;
#[cfg(windows)]
mod primitive;
#[cfg(all(windows, feature = "diagnostics"))]
mod profile;
pub mod query;
pub mod queue;
#[cfg(windows)]
mod queue2;
#[cfg(all(windows, feature = "diagnostics"))]
mod renderdoc;
pub mod resource;
pub mod root;
#[cfg(windows)]
mod sampler;
#[cfg(windows)]
pub mod shader_cache;
#[cfg(windows)]
mod shadow;
#[cfg(all(windows, feature = "diagnostics"))]
mod shot;
pub mod signature;
#[cfg(windows)]
mod spread;
#[cfg(windows)]
mod stats;
#[cfg(windows)]
mod texback;
#[cfg(windows)]
mod texture;
#[cfg(windows)]
mod tiles;
mod tuning;
#[cfg(windows)]
mod watch;

#[cfg(windows)]
pub use host::on_failure;
#[cfg(windows)]
pub use inplace::{title_memory_freeing, title_memory_section, title_memory_view};
#[cfg(windows)]
pub use watch::{
    title_memory_committed, title_memory_reading, title_memory_remapped, title_memory_remapping,
    title_memory_written, writing as kernel_will_write,
};
