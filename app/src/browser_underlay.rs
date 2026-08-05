//! Browser underlays: webviews composited behind the terminal.
//!
//! Two kinds of underlay exist, with strictly separated ownership:
//!
//! - **Ambience** (user-owned): at most one per window, at the very back,
//!   driven only by the ambience-URL setting / env hook (and the human
//!   `warpctrl browser --ambience` commands). Agents have no route to it.
//! - **Agent** (agent-owned): keyed by the owning pane's terminal session
//!   UUID. Created by the `browser.*` control plane / the `/mcp` endpoint,
//!   visible only while the owning pane's tab is frontmost in its window, so
//!   an agent's browsing is scoped to the tab it runs in.
//!
//! Lifecycle for agent underlays: torn down when the owning pane closes, when
//! idle past [`AGENT_IDLE_TTL`], or when evicted by [`MAX_AGENT_UNDERLAYS_PER_WINDOW`].
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ::settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::platform::BrowserUnderlayOwner;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, ViewHandle, WindowId};

use crate::pane_group::PaneId;
use crate::settings::{
    BrowserGlassOpacity, BrowserUnderlaySettings, BrowserUnderlaySettingsChangedEvent,
};
use crate::workspace::Workspace;

/// Environment variable that, when set (and the feature flag is enabled),
/// attaches an ambience underlay loading its URL to every new window.
/// Takes precedence over the ambience-URL setting.
const TEST_URL_ENV_VAR: &str = "WARP_BROWSER_UNDERLAY_TEST_URL";

/// Workspace-level tint painted over a visible agent underlay: near-transparent
/// so the page is fully visible through the window chrome, with per-pane fills
/// carrying legibility instead.
const GLASS_INVERSION_WORKSPACE_TINT_OPACITY: u8 = 10;

/// Background fill opacity for panes other than the agent's while an agent
/// underlay is visible: near-opaque so those terminals stay fully readable.
const NON_GLASS_PANE_FILL_OPACITY: u8 = 95;

/// Agent underlays idle longer than this are torn down by the sweeper.
pub const AGENT_IDLE_TTL: Duration = Duration::from_secs(15 * 60);

/// At most this many agent underlays per window; the least-recently-used one
/// is evicted to make room (each underlay is a WebKit process tree).
pub const MAX_AGENT_UNDERLAYS_PER_WINDOW: usize = 3;

/// How often the sweeper looks for idle agent underlays.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

// --- Ambience (user-owned) ---------------------------------------------------

/// Attaches the ambience underlay to `window_id`. Returns false when the
/// feature is disabled or the platform attach failed.
pub fn attach_ambience(window_id: WindowId, url: &str, ctx: &mut AppContext) -> bool {
    if !BrowserUnderlayState::feature_enabled() {
        return false;
    }
    if !ctx
        .windows()
        .attach_background_webview(window_id, &BrowserUnderlayOwner::Ambience, url)
    {
        return false;
    }
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_ambience_attached(window_id, url.to_owned(), ctx);
    });
    true
}

/// Detaches the ambience underlay from `window_id`.
pub fn detach_ambience(window_id: WindowId, ctx: &mut AppContext) {
    ctx.windows()
        .detach_background_webview(window_id, &BrowserUnderlayOwner::Ambience);
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_ambience_detached(window_id, ctx);
    });
}

/// Navigates the ambience underlay of `window_id`. Returns false when none is
/// attached.
pub fn navigate_ambience(window_id: WindowId, url: &str, ctx: &mut AppContext) -> bool {
    if !BrowserUnderlayState::as_ref(ctx).ambience_attached(window_id) {
        return false;
    }
    ctx.windows()
        .navigate_background_webview(window_id, &BrowserUnderlayOwner::Ambience, url);
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_ambience_attached(window_id, url.to_owned(), ctx);
    });
    true
}

/// Attaches an ambience underlay to a newly-opened window when
/// [`TEST_URL_ENV_VAR`] or the ambience-URL setting is set.
pub fn maybe_attach_default_underlay(window_id: WindowId, ctx: &mut AppContext) {
    if !BrowserUnderlayState::feature_enabled() {
        return;
    }
    if let Ok(url) = std::env::var(TEST_URL_ENV_VAR)
        && !url.is_empty()
    {
        attach_ambience(window_id, &url, ctx);
        return;
    }
    let ambience_url = BrowserUnderlaySettings::as_ref(ctx)
        .ambience_url
        .value()
        .clone();
    if !ambience_url.is_empty() {
        attach_ambience(window_id, &ambience_url, ctx);
    }
}

/// Registers runtime wiring for the browser underlay. Call once at app
/// startup, after settings registration.
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
                // window's observers so they repaint.
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
/// skipped. Agent underlays are unaffected.
fn apply_ambience_url_setting(ctx: &mut AppContext) {
    let url = BrowserUnderlaySettings::as_ref(ctx)
        .ambience_url
        .value()
        .clone();
    let window_ids: Vec<WindowId> = ctx.window_ids().collect();
    for window_id in window_ids {
        if url.is_empty() {
            if BrowserUnderlayState::as_ref(ctx).ambience_attached(window_id) {
                detach_ambience(window_id, ctx);
            }
        } else if !navigate_ambience(window_id, &url, ctx) {
            attach_ambience(window_id, &url, ctx);
        }
    }
}

// --- Agent underlays (pane-owned) --------------------------------------------

/// Where an agent's pane currently lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaneLocation {
    window_id: WindowId,
    /// Whether the pane's tab is the active tab of its window.
    in_active_tab: bool,
}

/// Finds the window (and tab visibility) of `pane_id` by scanning workspaces.
fn locate_pane(pane_id: PaneId, ctx: &AppContext) -> Option<PaneLocation> {
    for window_id in ctx.window_ids() {
        let Some(workspace) = ctx
            .views_of_type::<Workspace>(window_id)
            .and_then(|workspaces| workspaces.into_iter().next())
        else {
            continue;
        };
        let location = workspace.read(ctx, |workspace, ctx| {
            let active_index = workspace.active_tab_index();
            for (index, pane_group) in workspace.tab_views().enumerate() {
                let contains = pane_group
                    .read(ctx, |pane_group, _| pane_group.visible_pane_ids())
                    .contains(&pane_id);
                if contains {
                    return Some(PaneLocation {
                        window_id,
                        in_active_tab: index == active_index,
                    });
                }
            }
            None
        });
        if location.is_some() {
            return location;
        }
    }
    None
}

/// Resolves a terminal pane by its persistent session UUID across all windows.
pub fn find_pane_by_session_uuid(session_uuid: &str, ctx: &AppContext) -> Option<PaneId> {
    let Ok(uuid_bytes) = hex_decode(session_uuid) else {
        return None;
    };
    for window_id in ctx.window_ids() {
        let Some(workspace) = ctx
            .views_of_type::<Workspace>(window_id)
            .and_then(|workspaces| workspaces.into_iter().next())
        else {
            continue;
        };
        let found = workspace.read(ctx, |workspace, ctx| {
            for pane_group in workspace.tab_views() {
                let found = pane_group.read(ctx, |pane_group, _| {
                    pane_group.find_terminal_pane_by_session_uuid(&uuid_bytes)
                });
                if found.is_some() {
                    return found;
                }
            }
            None
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

fn hex_decode(input: &str) -> Result<Vec<u8>, ()> {
    if input.len() % 2 != 0 {
        return Err(());
    }
    (0..input.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

/// Attaches (or navigates) the agent underlay owned by the pane with
/// `pane_key` (terminal session UUID, hex) and loads `url`. Enforces the
/// per-window cap by evicting the least-recently-used agent underlay.
pub fn agent_attach(pane_key: &str, url: &str, ctx: &mut AppContext) -> Result<(), String> {
    if !BrowserUnderlayState::feature_enabled() {
        return Err("the browser underlay feature is not enabled".to_owned());
    }
    let pane_id = find_pane_by_session_uuid(pane_key, ctx)
        .ok_or_else(|| format!("no pane with terminal session uuid {pane_key}"))?;
    let location = locate_pane(pane_id, ctx)
        .ok_or_else(|| "the bound pane is not part of any window".to_owned())?;

    // Enforce the per-window cap before adding another WebKit process tree.
    let evict = BrowserUnderlayState::as_ref(ctx).agent_eviction_candidate(location.window_id);
    if let Some(evicted_key) = evict {
        agent_detach(&evicted_key, ctx);
    }

    let owner = BrowserUnderlayOwner::Agent(pane_key.to_owned());
    if !ctx
        .windows()
        .attach_background_webview(location.window_id, &owner, url)
    {
        return Err("the pane's window cannot host a browser underlay".to_owned());
    }
    ctx.windows().set_background_webview_visible(
        location.window_id,
        &owner,
        location.in_active_tab,
    );
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.upsert_agent(
            pane_key.to_owned(),
            pane_id,
            location.window_id,
            url.to_owned(),
            location.in_active_tab,
            ctx,
        );
    });
    if ctx.has_singleton_model::<AgentBrowserSweeper>() {
        AgentBrowserSweeper::handle(ctx).update(ctx, |sweeper, ctx| {
            sweeper.ensure_scheduled(ctx);
        });
    }
    Ok(())
}

/// Navigates the agent underlay owned by `pane_key`.
pub fn agent_navigate(pane_key: &str, url: &str, ctx: &mut AppContext) -> Result<(), String> {
    let window_id = BrowserUnderlayState::as_ref(ctx)
        .agent(pane_key)
        .map(|agent| agent.window_id)
        .ok_or_else(|| format!("no agent browser attached for pane {pane_key}"))?;
    ctx.windows().navigate_background_webview(
        window_id,
        &BrowserUnderlayOwner::Agent(pane_key.to_owned()),
        url,
    );
    BrowserUnderlayState::handle(ctx).update(ctx, |state, _| {
        state.touch_agent(pane_key, Some(url.to_owned()));
    });
    Ok(())
}

/// Detaches the agent underlay owned by `pane_key`, if any.
pub fn agent_detach(pane_key: &str, ctx: &mut AppContext) {
    let window_id = BrowserUnderlayState::as_ref(ctx)
        .agent(pane_key)
        .map(|agent| agent.window_id);
    if let Some(window_id) = window_id {
        ctx.windows().detach_background_webview(
            window_id,
            &BrowserUnderlayOwner::Agent(pane_key.to_owned()),
        );
    }
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.remove_agent(pane_key, ctx);
    });
}

/// Re-resolves every agent underlay's pane location and applies visibility:
/// an agent underlay is visible only while its pane's tab is frontmost in its
/// window. Panes that moved windows get their underlay re-homed (the page
/// reloads); panes that no longer exist get their underlay detached.
///
/// Called from the tab-activation choke point, pane cleanup, and window close.
pub fn sync_agent_visibility(ctx: &mut AppContext) {
    if !ctx.has_singleton_model::<BrowserUnderlayState>() {
        return;
    }
    let agents: Vec<(String, PaneId, WindowId, String, bool)> = BrowserUnderlayState::as_ref(ctx)
        .agents()
        .map(|agent| {
            (
                agent.pane_key.clone(),
                agent.pane_id,
                agent.window_id,
                agent.url.clone(),
                agent.visible,
            )
        })
        .collect();
    for (pane_key, pane_id, window_id, url, was_visible) in agents {
        let owner = BrowserUnderlayOwner::Agent(pane_key.clone());
        match locate_pane(pane_id, ctx) {
            None => {
                // Pane is gone (closed, or its window is closing).
                agent_detach(&pane_key, ctx);
            }
            Some(location) if location.window_id != window_id => {
                // The pane moved to another window: re-home the underlay.
                // WKWebViews cannot move between windows, so this reloads the
                // page in the new window.
                ctx.windows().detach_background_webview(window_id, &owner);
                if ctx
                    .windows()
                    .attach_background_webview(location.window_id, &owner, &url)
                {
                    ctx.windows().set_background_webview_visible(
                        location.window_id,
                        &owner,
                        location.in_active_tab,
                    );
                    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                        state.agent_moved(
                            &pane_key,
                            location.window_id,
                            location.in_active_tab,
                            ctx,
                        );
                    });
                } else {
                    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                        state.remove_agent(&pane_key, ctx);
                    });
                }
            }
            Some(location) => {
                if location.in_active_tab != was_visible {
                    ctx.windows().set_background_webview_visible(
                        window_id,
                        &owner,
                        location.in_active_tab,
                    );
                    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                        state.set_agent_visible(&pane_key, location.in_active_tab, ctx);
                    });
                }
            }
        }
    }
}

/// Cleanup hook: a pane was destroyed and its `PaneId` is now invalid.
pub fn pane_closed(pane_id: PaneId, ctx: &mut AppContext) {
    if !ctx.has_singleton_model::<BrowserUnderlayState>() {
        return;
    }
    let pane_key = BrowserUnderlayState::as_ref(ctx)
        .agents()
        .find(|agent| agent.pane_id == pane_id)
        .map(|agent| agent.pane_key.clone());
    if let Some(pane_key) = pane_key {
        agent_detach(&pane_key, ctx);
    }
}

/// Cleanup hook: a window closed; its platform webviews die with it.
pub fn window_closed(window_id: WindowId, ctx: &mut AppContext) {
    if !ctx.has_singleton_model::<BrowserUnderlayState>() {
        return;
    }
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.forget_window(window_id, ctx);
    });
}

/// Detaches agent underlays idle past [`AGENT_IDLE_TTL`]. Returns whether any
/// agent underlays remain (for the sweeper's rescheduling decision).
pub fn sweep_idle_agents(ctx: &mut AppContext) -> bool {
    let now = Instant::now();
    let idle: Vec<String> = BrowserUnderlayState::as_ref(ctx)
        .agents()
        .filter(|agent| now.duration_since(agent.last_used) > AGENT_IDLE_TTL)
        .map(|agent| agent.pane_key.clone())
        .collect();
    for pane_key in idle {
        log::info!("detaching idle agent browser underlay for pane {pane_key}");
        agent_detach(&pane_key, ctx);
    }
    BrowserUnderlayState::as_ref(ctx).agents().next().is_some()
}

// --- Interactive-mode hotkeys -------------------------------------------------

/// Handles an interactive-mode hotkey reported by the platform's event
/// monitor, targeting the window's visible underlay (ambience or agent).
pub fn handle_hotkey(
    window_id: WindowId,
    owner: BrowserUnderlayOwner,
    event: warpui::platform::BrowserUnderlayHotkey,
    ctx: &mut AppContext,
) {
    let effective = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.apply_hotkey(window_id, &owner, event, ctx)
    });
    if let Some(effective) = effective {
        ctx.windows()
            .set_background_webview_interactive(window_id, &owner, effective);
    }
}

// --- Rendering ---------------------------------------------------------------

/// What (if anything) renders behind a window's active tab right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisibleUnderlay {
    /// The user's ambience underlay: the whole tab renders as glass.
    Ambience,
    /// An agent underlay: the agent's pane is a glass porthole, other panes
    /// stay near-opaque.
    Agent { pane_id: PaneId, glass: bool },
}

/// The underlay visible in `window_id`'s active tab, if any.
pub fn visible_underlay(window_id: WindowId, app: &AppContext) -> Option<VisibleUnderlay> {
    if !app.has_singleton_model::<BrowserUnderlayState>() {
        return None;
    }
    let state = BrowserUnderlayState::as_ref(app);
    if let Some(agent) = state
        .agents()
        .find(|agent| agent.window_id == window_id && agent.visible)
    {
        return Some(VisibleUnderlay::Agent {
            pane_id: agent.pane_id,
            glass: agent.glass,
        });
    }
    if state.ambience_attached(window_id) {
        return Some(VisibleUnderlay::Ambience);
    }
    None
}

/// The effective glass fill opacity for `window_id`: the visible agent's
/// override when set, otherwise the `glass_opacity` setting.
pub fn glass_fill_opacity(window_id: WindowId, app: &AppContext) -> u8 {
    let state = BrowserUnderlayState::as_ref(app);
    state
        .agents()
        .find(|agent| agent.window_id == window_id && agent.visible)
        .and_then(|agent| agent.glass_opacity_override)
        .unwrap_or_else(|| *BrowserUnderlaySettings::as_ref(app).glass_opacity)
        .min(BrowserGlassOpacity::MAX)
}

/// The workspace-level background fill opacity while an underlay is visible.
pub fn workspace_fill_opacity(window_id: WindowId, app: &AppContext) -> u8 {
    match visible_underlay(window_id, app) {
        Some(VisibleUnderlay::Agent { glass: true, .. }) => GLASS_INVERSION_WORKSPACE_TINT_OPACITY,
        _ => glass_fill_opacity(window_id, app),
    }
}

/// The background fill opacity a pane should paint, or `None` when the pane
/// paints no background of its own.
pub fn pane_fill_opacity(window_id: WindowId, pane_id: PaneId, app: &AppContext) -> Option<u8> {
    match visible_underlay(window_id, app)? {
        VisibleUnderlay::Ambience => None,
        VisibleUnderlay::Agent { glass: false, .. } => None,
        VisibleUnderlay::Agent {
            pane_id: agent_pane,
            glass: true,
        } => {
            if pane_id == agent_pane {
                Some(glass_fill_opacity(window_id, app))
            } else {
                Some(NON_GLASS_PANE_FILL_OPACITY)
            }
        }
    }
}

/// Whether input currently reaches the visible underlay of `window_id`
/// (drives the interactive-mode indicator).
pub fn effective_interactive(window_id: WindowId, app: &AppContext) -> bool {
    let state = BrowserUnderlayState::as_ref(app);
    if let Some(agent) = state
        .agents()
        .find(|agent| agent.window_id == window_id && agent.visible)
    {
        return agent.interactive || agent.interactive_hold;
    }
    state
        .ambience(window_id)
        .is_some_and(|ambience| ambience.interactive || ambience.interactive_hold)
}

// --- State model --------------------------------------------------------------

/// The user-owned ambience underlay of one window.
#[derive(Debug, Clone, Default)]
pub struct AmbienceUnderlay {
    pub url: String,
    pub interactive: bool,
    pub interactive_hold: bool,
}

/// An agent-owned underlay, keyed by its pane's terminal session UUID.
#[derive(Debug, Clone)]
pub struct AgentUnderlay {
    pub pane_key: String,
    pub pane_id: PaneId,
    pub window_id: WindowId,
    pub url: String,
    /// Whether the underlay is currently unhidden (its pane's tab frontmost).
    pub visible: bool,
    /// Whether the agent's pane renders as a glass porthole (vs whole-tab
    /// glass over the agent's page).
    pub glass: bool,
    pub glass_opacity_override: Option<u8>,
    pub interactive: bool,
    pub interactive_hold: bool,
    /// Last control-plane/MCP operation touching this underlay (idle TTL).
    pub last_used: Instant,
}

/// Singleton model tracking all browser underlays. The native webviews live
/// in the macOS platform layer; this model mirrors that state so rendering
/// (glass fills, indicators) can react without platform calls during layout.
#[derive(Debug, Default)]
pub struct BrowserUnderlayState {
    ambience: HashMap<WindowId, AmbienceUnderlay>,
    agents: HashMap<String, AgentUnderlay>,
}

#[derive(Debug)]
pub enum BrowserUnderlayEvent {
    /// Underlay state for this window was attached, detached, or reconfigured.
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

    pub fn ambience(&self, window_id: WindowId) -> Option<&AmbienceUnderlay> {
        self.ambience.get(&window_id)
    }

    pub fn ambience_attached(&self, window_id: WindowId) -> bool {
        self.ambience.contains_key(&window_id)
    }

    pub fn agent(&self, pane_key: &str) -> Option<&AgentUnderlay> {
        self.agents.get(pane_key)
    }

    pub fn agents(&self) -> impl Iterator<Item = &AgentUnderlay> {
        self.agents.values()
    }

    /// The least-recently-used agent underlay in `window_id` when the window
    /// is at [`MAX_AGENT_UNDERLAYS_PER_WINDOW`], else `None`.
    pub fn agent_eviction_candidate(&self, window_id: WindowId) -> Option<String> {
        let in_window: Vec<&AgentUnderlay> = self
            .agents
            .values()
            .filter(|agent| agent.window_id == window_id)
            .collect();
        if in_window.len() < MAX_AGENT_UNDERLAYS_PER_WINDOW {
            return None;
        }
        in_window
            .into_iter()
            .min_by_key(|agent| agent.last_used)
            .map(|agent| agent.pane_key.clone())
    }

    pub fn set_ambience_attached(
        &mut self,
        window_id: WindowId,
        url: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.ambience.entry(window_id).or_default().url = url;
        ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        ctx.notify();
    }

    pub fn set_ambience_detached(&mut self, window_id: WindowId, ctx: &mut ModelContext<Self>) {
        if self.ambience.remove(&window_id).is_some() {
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
            ctx.notify();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_agent(
        &mut self,
        pane_key: String,
        pane_id: PaneId,
        window_id: WindowId,
        url: String,
        visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let agent = self
            .agents
            .entry(pane_key.clone())
            .or_insert_with(|| AgentUnderlay {
                pane_key,
                pane_id,
                window_id,
                url: String::new(),
                visible,
                glass: true,
                glass_opacity_override: None,
                interactive: false,
                interactive_hold: false,
                last_used: Instant::now(),
            });
        agent.pane_id = pane_id;
        agent.window_id = window_id;
        agent.url = url;
        agent.visible = visible;
        agent.last_used = Instant::now();
        ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        ctx.notify();
    }

    pub fn remove_agent(&mut self, pane_key: &str, ctx: &mut ModelContext<Self>) {
        if let Some(agent) = self.agents.remove(pane_key) {
            ctx.emit(BrowserUnderlayEvent::Changed(agent.window_id));
            ctx.notify();
        }
    }

    pub fn agent_moved(
        &mut self,
        pane_key: &str,
        window_id: WindowId,
        visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(agent) = self.agents.get_mut(pane_key) {
            let old_window = agent.window_id;
            agent.window_id = window_id;
            agent.visible = visible;
            // Re-homing resets interactive state (the webview was recreated).
            agent.interactive = false;
            agent.interactive_hold = false;
            ctx.emit(BrowserUnderlayEvent::Changed(old_window));
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
            ctx.notify();
        }
    }

    pub fn set_agent_visible(
        &mut self,
        pane_key: &str,
        visible: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(agent) = self.agents.get_mut(pane_key) {
            agent.visible = visible;
            if !visible {
                // Hidden underlays never own input (the platform enforces the
                // same rule).
                agent.interactive = false;
                agent.interactive_hold = false;
            }
            ctx.emit(BrowserUnderlayEvent::Changed(agent.window_id));
            ctx.notify();
        }
    }

    /// Records a control-plane/MCP operation on the agent underlay and
    /// optionally updates its URL.
    pub fn touch_agent(&mut self, pane_key: &str, url: Option<String>) {
        if let Some(agent) = self.agents.get_mut(pane_key) {
            agent.last_used = Instant::now();
            if let Some(url) = url {
                agent.url = url;
            }
        }
    }

    /// Sets the agent's glass porthole mode / opacity override. Returns false
    /// when no such agent underlay exists.
    pub fn set_agent_glass(
        &mut self,
        pane_key: &str,
        glass: bool,
        opacity_override: Option<u8>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let Some(agent) = self.agents.get_mut(pane_key) else {
            return false;
        };
        agent.glass = glass;
        if let Some(opacity) = opacity_override {
            agent.glass_opacity_override = Some(opacity.min(BrowserGlassOpacity::MAX));
        }
        agent.last_used = Instant::now();
        ctx.emit(BrowserUnderlayEvent::Changed(agent.window_id));
        ctx.notify();
        true
    }

    /// Sets latched interactive mode on an underlay. Returns the new
    /// effective interactive state, or `None` when it doesn't exist.
    pub fn set_interactive(
        &mut self,
        window_id: WindowId,
        owner: &BrowserUnderlayOwner,
        interactive: bool,
        ctx: &mut ModelContext<Self>,
    ) -> Option<bool> {
        let effective = match owner {
            BrowserUnderlayOwner::Ambience => {
                let ambience = self.ambience.get_mut(&window_id)?;
                ambience.interactive = interactive;
                ambience.interactive || ambience.interactive_hold
            }
            BrowserUnderlayOwner::Agent(pane_key) => {
                let agent = self.agents.get_mut(pane_key)?;
                agent.interactive = interactive;
                agent.last_used = Instant::now();
                agent.interactive || agent.interactive_hold
            }
        };
        ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        ctx.notify();
        Some(effective)
    }

    /// Applies an interactive-mode hotkey gesture. Returns the new effective
    /// interactive state, or `None` when the underlay doesn't exist.
    pub fn apply_hotkey(
        &mut self,
        window_id: WindowId,
        owner: &BrowserUnderlayOwner,
        event: warpui::platform::BrowserUnderlayHotkey,
        ctx: &mut ModelContext<Self>,
    ) -> Option<bool> {
        use warpui::platform::BrowserUnderlayHotkey;

        let (interactive, hold) = match owner {
            BrowserUnderlayOwner::Ambience => {
                let ambience = self.ambience.get_mut(&window_id)?;
                (&mut ambience.interactive, &mut ambience.interactive_hold)
            }
            BrowserUnderlayOwner::Agent(pane_key) => {
                let agent = self.agents.get_mut(pane_key)?;
                agent.last_used = Instant::now();
                (&mut agent.interactive, &mut agent.interactive_hold)
            }
        };
        match event {
            BrowserUnderlayHotkey::Toggle => *interactive = !*interactive,
            BrowserUnderlayHotkey::HoldStart => *hold = true,
            BrowserUnderlayHotkey::HoldEnd => *hold = false,
            BrowserUnderlayHotkey::ForceOff => {
                *interactive = false;
                *hold = false;
            }
        }
        let effective = *interactive || *hold;
        ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        ctx.notify();
        Some(effective)
    }

    /// Drops all state for a closed window (platform views die with it).
    pub fn forget_window(&mut self, window_id: WindowId, ctx: &mut ModelContext<Self>) {
        let had_ambience = self.ambience.remove(&window_id).is_some();
        let agent_keys: Vec<String> = self
            .agents
            .values()
            .filter(|agent| agent.window_id == window_id)
            .map(|agent| agent.pane_key.clone())
            .collect();
        for key in &agent_keys {
            self.agents.remove(key);
        }
        if had_ambience || !agent_keys.is_empty() {
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
            ctx.notify();
        }
    }

    /// Emits [`BrowserUnderlayEvent::Changed`] for every window with any
    /// underlay, so observers repaint after an external change (e.g. the
    /// glass-opacity setting).
    pub fn emit_changed_for_all(&mut self, ctx: &mut ModelContext<Self>) {
        let mut windows: HashSet<WindowId> = self.ambience.keys().copied().collect();
        windows.extend(self.agents.values().map(|agent| agent.window_id));
        for window_id in windows {
            ctx.emit(BrowserUnderlayEvent::Changed(window_id));
        }
        ctx.notify();
    }
}

impl SingletonEntity for BrowserUnderlayState {}

impl Entity for BrowserUnderlayState {
    type Event = BrowserUnderlayEvent;
}

// --- Idle sweeper -------------------------------------------------------------

/// Periodically detaches idle agent underlays (each is a WebKit process
/// tree). Mirrors `LanguageServerShutdownManager`: a self-rescheduling timer
/// that runs only while there is something to sweep.
pub struct AgentBrowserSweeper {
    in_progress: Option<futures::stream::AbortHandle>,
}

impl AgentBrowserSweeper {
    pub fn new() -> Self {
        Self { in_progress: None }
    }

    /// Starts the sweep loop if it isn't already running.
    pub fn ensure_scheduled(&mut self, ctx: &mut ModelContext<Self>) {
        if self.in_progress.is_some() {
            return;
        }
        self.schedule(ctx);
    }

    fn schedule(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(sweep) = self.in_progress.take() {
            sweep.abort();
        }
        self.in_progress = Some(
            ctx.spawn(
                async {
                    warpui::r#async::Timer::after(SWEEP_INTERVAL).await;
                },
                |me, _, ctx| {
                    if sweep_idle_agents(ctx) {
                        me.schedule(ctx);
                    } else {
                        me.in_progress = None;
                    }
                },
            )
            .abort_handle(),
        );
    }
}

impl Default for AgentBrowserSweeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for AgentBrowserSweeper {
    type Event = ();
}

impl SingletonEntity for AgentBrowserSweeper {}

// Suppress an unused-import warning on non-mac targets where the platform
// calls compile to no-ops.
#[allow(unused)]
fn _owner_type_check(_: &ViewHandle<Workspace>) {}

#[cfg(test)]
#[path = "browser_underlay_tests.rs"]
mod tests;
