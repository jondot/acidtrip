//! acidtrip core: document model, transactions, history, tools and rendering.

pub mod charsets;
pub mod color;
pub mod cp437;
pub mod filters;
pub mod history;
pub mod mirror;
pub mod model;
pub mod render;
pub mod replay;
pub mod tools;
pub mod tx;

pub use color::{Color, Palette};
pub use history::History;
pub use model::{
    Canvas, Cell, Clip, DocKind, DocMeta, Document, ExportPreset, ExportSettings, Frame, FrameSet, Frames, Grid, Layer,
    LayerKind, SauceMeta,
};
pub use tx::{Transaction, TxBuilder};
