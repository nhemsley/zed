use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use calloop::{EventLoop, LoopHandle};
use util::ResultExt;

use crate::platform::linux::LinuxClient;
use crate::platform::{LinuxCommon, PlatformWindow};
use crate::{
    AnyWindowHandle, ClipboardItem, CursorStyle, DisplayId, LinuxKeyboardLayout, PlatformDisplay,
    PlatformKeyboardLayout, WindowParams,
};

use super::{TexturedSurfaceDisplay, TexturedSurfaceWindow};

pub struct TexturedSurfaceClientState {
    pub(crate) _loop_handle: LoopHandle<'static, TexturedSurfaceClient>,
    pub(crate) event_loop: Option<calloop::EventLoop<'static, TexturedSurfaceClient>>,
    pub(crate) common: LinuxCommon,
}

#[derive(Clone)]
pub(crate) struct TexturedSurfaceClient(Rc<RefCell<TexturedSurfaceClientState>>);

impl TexturedSurfaceClient {
    pub(crate) fn new() -> Self {
        let event_loop = EventLoop::try_new().unwrap();

        let (common, main_receiver) = LinuxCommon::new(event_loop.get_signal());

        let handle = event_loop.handle();

        handle
            .insert_source(main_receiver, |event, _, _: &mut TexturedSurfaceClient| {
                if let calloop::channel::Event::Msg(runnable) = event {
                    match runnable {
                        crate::RunnableVariant::Meta(runnable) => runnable.run(),
                        crate::RunnableVariant::Compat(runnable) => runnable.run(),
                    };
                }
            })
            .ok();

        TexturedSurfaceClient(Rc::new(RefCell::new(TexturedSurfaceClientState {
            event_loop: Some(event_loop),
            _loop_handle: handle,
            common,
        })))
    }
}

impl LinuxClient for TexturedSurfaceClient {
    fn with_common<R>(&self, f: impl FnOnce(&mut LinuxCommon) -> R) -> R {
        f(&mut self.0.borrow_mut().common)
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(LinuxKeyboardLayout::new("us".into()))
    }

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        vec![Rc::new(TexturedSurfaceDisplay::new())]
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(TexturedSurfaceDisplay::new()))
    }

    fn display(&self, _id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(TexturedSurfaceDisplay::new()))
    }

    #[cfg(feature = "screen-capture")]
    fn is_screen_capture_supported(&self) -> bool {
        false
    }

    #[cfg(feature = "screen-capture")]
    fn screen_capture_sources(
        &self,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Vec<Rc<dyn crate::ScreenCaptureSource>>>>
    {
        let (mut tx, rx) = futures::channel::oneshot::channel();
        tx.send(Err(anyhow::anyhow!(
            "Textured surface mode does not support screen capture."
        )))
        .ok();
        rx
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        None
    }

    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        None
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        params: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        let window = TexturedSurfaceWindow::new(handle, params)?;
        Ok(Box::new(window))
    }

    fn compositor_name(&self) -> &'static str {
        "textured_surface"
    }

    fn set_cursor_style(&self, _style: CursorStyle) {}

    fn open_uri(&self, _uri: &str) {}

    fn reveal_path(&self, _path: PathBuf) {}

    fn write_to_primary(&self, _item: ClipboardItem) {}

    fn write_to_clipboard(&self, _item: ClipboardItem) {}

    fn read_from_primary(&self) -> Option<ClipboardItem> {
        None
    }

    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        None
    }

    fn run(&self) {
        let mut event_loop = self
            .0
            .borrow_mut()
            .event_loop
            .take()
            .expect("App is already running");

        event_loop.run(None, &mut self.clone(), |_| {}).log_err();
    }
}
