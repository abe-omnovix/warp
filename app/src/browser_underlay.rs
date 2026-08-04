use std::collections::HashMap;

use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, WindowId};

/// Environment variable that, when set (and the feature flag is enabled),
/// attaches a browser underlay loading its URL to every new window. Smoke-test
/// hook until the `browser.*` local-control actions land.
const TEST_URL_ENV_VAR: &str = "WARP_BROWSER_UNDERLAY_TEST_URL";

/// Attaches a browser underlay to `window_id` (platform webview + model
/// state). Returns false when the feature is disabled or the platform attach
/// failed.
pub fn attach_to_window(window_id: WindowId, url: &str, ctx: &mut AppContext) -> bool {
    if !BrowserUnderlayState::feature_enabled() {
        return false;
    }
    if !ctx.windows().attach_background_webview(window_id, url) {
        return false;
    }
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_attached(window_id, url.to_owned(), ctx);
    });
    true
}

/// Detaches the browser underlay from `window_id` (platform webview + model
/// state).
pub fn detach_from_window(window_id: WindowId, ctx: &mut AppContext) {
    ctx.windows().detach_background_webview(window_id);
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_detached(window_id, ctx);
    });
}

/// Attaches an underlay to a newly-opened window when [`TEST_URL_ENV_VAR`] is
/// set.
pub fn maybe_attach_test_underlay(window_id: WindowId, ctx: &mut AppContext) {
    if !BrowserUnderlayState::feature_enabled() {
        return;
    }
    if let Ok(url) = std::env::var(TEST_URL_ENV_VAR)
        && !url.is_empty()
    {
        attach_to_window(window_id, &url, ctx);
    }
}

/// Per-window state of an attached browser underlay.
#[derive(Debug, Clone)]
pub struct WindowUnderlay {
    pub url: String,
    pub interactive: bool,
}

/// Singleton model tracking which windows have a browser background underlay
/// attached. The native webview itself lives in the macOS platform layer
/// (`warpui::platform::Window::attach_background_webview` and friends); this
/// model mirrors that state so rendering (workspace glass, pane fills) can
/// react to it without platform calls during layout.
#[derive(Debug, Default)]
pub struct BrowserUnderlayState {
    windows: HashMap<WindowId, WindowUnderlay>,
}

#[derive(Debug)]
pub enum BrowserUnderlayEvent {
    /// The underlay for this window was attached, detached, or reconfigured.
    Changed(WindowId),
}

impl BrowserUnderlayState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the feature as a whole is available.
    pub fn feature_enabled() -> bool {
        cfg!(target_os = "macos") && FeatureFlag::BrowserUnderlay.is_enabled()
    }

    pub fn attached(&self, window_id: WindowId) -> Option<&WindowUnderlay> {
        self.windows.get(&window_id)
    }

    pub fn is_attached(&self, window_id: WindowId) -> bool {
        self.windows.contains_key(&window_id)
    }

    pub fn set_attached(&mut self, window_id: WindowId, url: String, ctx: &mut ModelContext<Self>) {
        let underlay = self.windows.entry(window_id).or_insert(WindowUnderlay {
            url: String::new(),
            interactive: false,
        });
        underlay.url = url;
        ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        ctx.notify();
    }

    pub fn set_interactive(
        &mut self,
        window_id: WindowId,
        interactive: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(underlay) = self.windows.get_mut(&window_id) {
            underlay.interactive = interactive;
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
            ctx.notify();
        }
    }

    pub fn set_detached(&mut self, window_id: WindowId, ctx: &mut ModelContext<Self>) {
        if self.windows.remove(&window_id).is_some() {
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
            ctx.notify();
        }
    }
}

impl SingletonEntity for BrowserUnderlayState {}

impl Entity for BrowserUnderlayState {
    type Event = BrowserUnderlayEvent;
}
