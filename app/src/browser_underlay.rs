use std::collections::{HashMap, HashSet};

use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, WindowId};

use crate::pane_group::PaneId;
use crate::settings::{
    BrowserGlassOpacity, BrowserUnderlaySettings, BrowserUnderlaySettingsChangedEvent,
};

/// Environment variable that, when set (and the feature flag is enabled),
/// attaches a browser underlay loading its URL to every new window. Smoke-test
/// hook until the `browser.*` local-control actions land; takes precedence
/// over the ambience-URL setting.
const TEST_URL_ENV_VAR: &str = "WARP_BROWSER_UNDERLAY_TEST_URL";

/// Workspace-level tint painted over the underlay while at least one pane is
/// glass: near-transparent so the page is fully visible through the window
/// chrome, with per-pane fills carrying legibility instead.
const GLASS_INVERSION_WORKSPACE_TINT_OPACITY: u8 = 10;

/// Background fill opacity for non-glass panes while some other pane in the
/// window is glass: near-opaque so those terminals stay fully readable.
const NON_GLASS_PANE_FILL_OPACITY: u8 = 95;

/// Attaches a browser underlay to `window_id` (platform webview + model
/// state). Returns false when the feature is disabled or the platform attach
/// failed (e.g. the window cannot host an underlay).
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

/// Navigates the underlay attached to `window_id` (platform webview + model
/// state). Returns false when no underlay is attached.
pub fn navigate_window(window_id: WindowId, url: &str, ctx: &mut AppContext) -> bool {
    if !BrowserUnderlayState::as_ref(ctx).is_attached(window_id) {
        return false;
    }
    ctx.windows().navigate_background_webview(window_id, url);
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_attached(window_id, url.to_owned(), ctx);
    });
    true
}

/// Attaches an underlay to a newly-opened window when [`TEST_URL_ENV_VAR`] or
/// the ambience-URL setting is set.
pub fn maybe_attach_default_underlay(window_id: WindowId, ctx: &mut AppContext) {
    if !BrowserUnderlayState::feature_enabled() {
        return;
    }
    if let Ok(url) = std::env::var(TEST_URL_ENV_VAR)
        && !url.is_empty()
    {
        attach_to_window(window_id, &url, ctx);
        return;
    }
    let ambience_url = BrowserUnderlaySettings::as_ref(ctx)
        .ambience_url
        .value()
        .clone();
    if !ambience_url.is_empty() {
        attach_to_window(window_id, &ambience_url, ctx);
    }
}

/// Registers runtime wiring for the browser underlay: applying ambience-URL
/// setting changes to open windows and re-rendering glass when the opacity
/// setting changes. Call once at app startup, after settings registration.
pub fn init(ctx: &mut AppContext) {
    if !BrowserUnderlayState::feature_enabled() {
        return;
    }
    ctx.subscribe_to_model(
        &BrowserUnderlaySettings::handle(ctx),
        |_, event, ctx| match event {
            BrowserUnderlaySettingsChangedEvent::BrowserAmbienceUrl { .. } => {
                apply_ambience_url_setting(ctx);
            }
            BrowserUnderlaySettingsChangedEvent::BrowserGlassOpacity { .. } => {
                // Glass fills read the setting at render time; nudge every
                // attached window's observers so they repaint.
                BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                    state.emit_changed_for_all(ctx);
                });
            }
        },
    );
}

/// Applies the current ambience-URL setting to every open window: attaches or
/// navigates when non-empty, detaches everywhere when cleared. Windows that
/// cannot host an underlay (e.g. panels) fail the platform attach and are
/// skipped.
fn apply_ambience_url_setting(ctx: &mut AppContext) {
    let url = BrowserUnderlaySettings::as_ref(ctx)
        .ambience_url
        .value()
        .clone();
    let window_ids: Vec<WindowId> = ctx.window_ids().collect();
    for window_id in window_ids {
        if url.is_empty() {
            if BrowserUnderlayState::as_ref(ctx).is_attached(window_id) {
                detach_from_window(window_id, ctx);
            }
        } else if !navigate_window(window_id, &url, ctx) {
            attach_to_window(window_id, &url, ctx);
        }
    }
}

/// The effective glass fill opacity for `window_id`: the per-window override
/// when set (via `pane.glass.set`), otherwise the `glass_opacity` setting.
///
/// Only meaningful while an underlay is attached — callers must ensure the
/// feature is enabled (the settings group is only registered then).
pub fn glass_fill_opacity(window_id: WindowId, app: &AppContext) -> u8 {
    let state = BrowserUnderlayState::as_ref(app);
    state
        .attached(window_id)
        .and_then(|underlay| underlay.glass_opacity_override)
        .unwrap_or_else(|| *BrowserUnderlaySettings::as_ref(app).glass_opacity)
        .min(BrowserGlassOpacity::MAX)
}

/// The workspace-level background fill opacity for a window with an underlay
/// attached. While any pane is glass the workspace drops to a faint tint and
/// per-pane fills take over; otherwise the whole window renders as glass.
pub fn workspace_fill_opacity(window_id: WindowId, app: &AppContext) -> u8 {
    if BrowserUnderlayState::as_ref(app).has_glass_panes(window_id) {
        GLASS_INVERSION_WORKSPACE_TINT_OPACITY
    } else {
        glass_fill_opacity(window_id, app)
    }
}

/// The background fill opacity a pane should paint, or `None` when the pane
/// paints no background of its own (no underlay attached, or window-level
/// glass with no per-pane portholes).
pub fn pane_fill_opacity(window_id: WindowId, pane_id: PaneId, app: &AppContext) -> Option<u8> {
    if !app.has_singleton_model::<BrowserUnderlayState>() {
        return None;
    }
    let state = BrowserUnderlayState::as_ref(app);
    let underlay = state.attached(window_id)?;
    if underlay.glass_panes.is_empty() {
        return None;
    }
    if underlay.glass_panes.contains(&pane_id) {
        Some(glass_fill_opacity(window_id, app))
    } else {
        Some(NON_GLASS_PANE_FILL_OPACITY)
    }
}

/// Per-window state of an attached browser underlay.
#[derive(Debug, Clone, Default)]
pub struct WindowUnderlay {
    pub url: String,
    pub interactive: bool,
    /// Panes rendered as translucent night-glass portholes onto the underlay.
    /// Empty ⇒ the whole window renders as glass (ambience mode).
    ///
    /// [`PaneId`]s are process-local and never persisted. Entries for panes
    /// that have since closed are inert (nothing renders them) and are
    /// discarded with the set on detach.
    pub glass_panes: HashSet<PaneId>,
    /// Per-window override of the `glass_opacity` setting (0-100).
    pub glass_opacity_override: Option<u8>,
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

    /// Whether at least one pane in `window_id` is rendered as glass.
    pub fn has_glass_panes(&self, window_id: WindowId) -> bool {
        self.windows
            .get(&window_id)
            .is_some_and(|underlay| !underlay.glass_panes.is_empty())
    }

    pub fn is_glass_pane(&self, window_id: WindowId, pane_id: PaneId) -> bool {
        self.windows
            .get(&window_id)
            .is_some_and(|underlay| underlay.glass_panes.contains(&pane_id))
    }

    pub fn set_attached(&mut self, window_id: WindowId, url: String, ctx: &mut ModelContext<Self>) {
        let underlay = self.windows.entry(window_id).or_default();
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

    /// Adds or removes `pane_id` from the window's glass set, optionally
    /// updating the window's glass-opacity override. No-op when no underlay is
    /// attached to the window. Returns whether the window has an underlay.
    pub fn set_pane_glass(
        &mut self,
        window_id: WindowId,
        pane_id: PaneId,
        enabled: bool,
        opacity_override: Option<u8>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(underlay) = self.windows.get_mut(&window_id) else {
            return false;
        };
        if enabled {
            underlay.glass_panes.insert(pane_id);
        } else {
            underlay.glass_panes.remove(&pane_id);
        }
        if let Some(opacity) = opacity_override {
            underlay.glass_opacity_override = Some(opacity.min(BrowserGlassOpacity::MAX));
        }
        ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        ctx.notify();
        true
    }

    pub fn set_detached(&mut self, window_id: WindowId, ctx: &mut ModelContext<Self>) {
        if self.windows.remove(&window_id).is_some() {
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
            ctx.notify();
        }
    }

    /// Emits [`BrowserUnderlayEvent::Changed`] for every attached window, so
    /// observers repaint after a change external to this model (e.g. the
    /// glass-opacity setting).
    pub fn emit_changed_for_all(&mut self, ctx: &mut ModelContext<Self>) {
        for window_id in self.windows.keys().copied().collect::<Vec<_>>() {
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        }
        ctx.notify();
    }
}

impl SingletonEntity for BrowserUnderlayState {}

impl Entity for BrowserUnderlayState {
    type Event = BrowserUnderlayEvent;
}

#[cfg(test)]
#[path = "browser_underlay_tests.rs"]
mod tests;
