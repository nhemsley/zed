use crate::{Bounds, DisplayId, PlatformDisplay, Pixels, Size, px};
use anyhow::Result;
use uuid::Uuid;

/// A virtual display for TexturedSurface windows.
/// This provides layout information without requiring a real display connection.
#[derive(Debug)]
pub struct TexturedSurfaceDisplay {
    bounds: Bounds<Pixels>,
    uuid: Uuid,
}

impl TexturedSurfaceDisplay {
    pub fn new() -> Self {
        Self::with_size(Size {
            width: px(1920.0),
            height: px(1080.0),
        })
    }

    pub fn with_size(size: Size<Pixels>) -> Self {
        Self {
            bounds: Bounds {
                origin: Default::default(),
                size,
            },
            uuid: Uuid::from_bytes([0; 16]),
        }
    }
}

impl PlatformDisplay for TexturedSurfaceDisplay {
    fn id(&self) -> DisplayId {
        DisplayId(0)
    }

    fn uuid(&self) -> Result<Uuid> {
        Ok(self.uuid)
    }

    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }
}
