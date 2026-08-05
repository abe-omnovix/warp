use settings::{PrivatePreferences, PublicPreferences, Setting as _, SettingsManager};
use warpui::platform::BrowserUnderlayOwner;
use warpui::{App, AppContext, WindowId};
use warpui_extras::user_preferences;

use super::*;
use crate::pane_group::TerminalPaneId;
use crate::settings::BrowserUnderlaySettings;

/// Registers in-memory preference stores, the settings manager, the
/// browser-underlay settings group, and the state singleton.
fn init_test_app(ctx: &mut AppContext) {
    ctx.add_singleton_model(move |_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(move |_| -> PrivatePreferences {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(|_| SettingsManager::default());
    BrowserUnderlaySettings::register(ctx);
    ctx.add_singleton_model(|_| BrowserUnderlayState::new());
}

fn test_pane_id() -> PaneId {
    PaneId::from(TerminalPaneId::dummy_terminal_pane_id())
}

fn attach_ambience_state(window_id: WindowId, ctx: &mut AppContext) {
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_ambience_attached(window_id, "https://ambience.example".to_owned(), ctx);
    });
}

fn attach_agent_state(
    pane_key: &str,
    pane_id: PaneId,
    window_id: WindowId,
    visible: bool,
    ctx: &mut AppContext,
) {
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.upsert_agent(
            pane_key.to_owned(),
            pane_id,
            window_id,
            "https://agent.example".to_owned(),
            visible,
            ctx,
        );
    });
}

#[test]
fn ambience_renders_whole_window_glass() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach_ambience_state(window_id, ctx);

            assert_eq!(
                visible_underlay(window_id, ctx),
                Some(VisibleUnderlay::Ambience)
            );
            // 55 is the `glass_opacity` setting default.
            assert_eq!(workspace_fill_opacity(window_id, ctx), 55);
            assert_eq!(pane_fill_opacity(window_id, test_pane_id(), ctx), None);
        });
    });
}

#[test]
fn visible_agent_takes_priority_over_ambience() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach_ambience_state(window_id, ctx);
            attach_agent_state("abc123", pane_id, window_id, true, ctx);

            assert_eq!(
                visible_underlay(window_id, ctx),
                Some(VisibleUnderlay::Agent {
                    pane_id,
                    glass: true
                })
            );
            assert_eq!(workspace_fill_opacity(window_id, ctx), 10);
        });
    });
}

#[test]
fn hidden_agent_falls_back_to_ambience() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach_ambience_state(window_id, ctx);
            attach_agent_state("abc123", test_pane_id(), window_id, false, ctx);

            assert_eq!(
                visible_underlay(window_id, ctx),
                Some(VisibleUnderlay::Ambience)
            );
        });
    });
}

#[test]
fn agent_pane_is_glass_porthole_and_others_near_opaque() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let agent_pane = test_pane_id();
            let other_pane = test_pane_id();
            attach_agent_state("abc123", agent_pane, window_id, true, ctx);

            assert_eq!(pane_fill_opacity(window_id, agent_pane, ctx), Some(55));
            assert_eq!(pane_fill_opacity(window_id, other_pane, ctx), Some(95));
        });
    });
}

#[test]
fn agent_glass_off_renders_whole_tab_glass() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let agent_pane = test_pane_id();
            attach_agent_state("abc123", agent_pane, window_id, true, ctx);
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                assert!(state.set_agent_glass("abc123", false, None, ctx));
            });

            assert_eq!(pane_fill_opacity(window_id, agent_pane, ctx), None);
            assert_eq!(workspace_fill_opacity(window_id, ctx), 55);
        });
    });
}

#[test]
fn agent_glass_opacity_override_beats_setting() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let agent_pane = test_pane_id();
            attach_agent_state("abc123", agent_pane, window_id, true, ctx);
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_agent_glass("abc123", true, Some(30), ctx);
            });

            assert_eq!(pane_fill_opacity(window_id, agent_pane, ctx), Some(30));
        });
    });
}

#[test]
fn eviction_candidate_appears_at_cap() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            for key in ["a1", "a2"] {
                attach_agent_state(key, test_pane_id(), window_id, false, ctx);
            }
            assert_eq!(
                BrowserUnderlayState::as_ref(ctx).agent_eviction_candidate(window_id),
                None
            );

            attach_agent_state("a3", test_pane_id(), window_id, false, ctx);

            // "a1" was attached first, so it is the least recently used.
            assert_eq!(
                BrowserUnderlayState::as_ref(ctx).agent_eviction_candidate(window_id),
                Some("a1".to_owned())
            );
        });
    });
}

#[test]
fn forget_window_drops_ambience_and_agents() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach_ambience_state(window_id, ctx);
            attach_agent_state("abc123", test_pane_id(), window_id, true, ctx);

            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.forget_window(window_id, ctx);
            });

            assert_eq!(visible_underlay(window_id, ctx), None);
            assert!(BrowserUnderlayState::as_ref(ctx).agent("abc123").is_none());
        });
    });
}

#[test]
fn hiding_an_agent_clears_its_interactive_state() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach_agent_state("abc123", test_pane_id(), window_id, true, ctx);
            let owner = BrowserUnderlayOwner::Agent("abc123".to_owned());
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_interactive(window_id, &owner, true, ctx);
            });
            assert!(effective_interactive(window_id, ctx));

            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_agent_visible("abc123", false, ctx);
            });

            assert!(!effective_interactive(window_id, ctx));
        });
    });
}

#[test]
fn toggle_hotkey_flips_latched_interactive_per_owner() {
    use warpui::platform::BrowserUnderlayHotkey::Toggle;

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach_ambience_state(window_id, ctx);
            let owner = BrowserUnderlayOwner::Ambience;

            let apply = |ctx: &mut AppContext| {
                BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                    state.apply_hotkey(window_id, &owner, Toggle, ctx)
                })
            };
            assert_eq!(apply(ctx), Some(true));
            assert_eq!(apply(ctx), Some(false));
        });
    });
}

#[test]
fn hold_hotkey_is_momentary_and_latch_survives_release() {
    use warpui::platform::BrowserUnderlayHotkey::{HoldEnd, HoldStart, Toggle};

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach_agent_state("abc123", test_pane_id(), window_id, true, ctx);
            let owner = BrowserUnderlayOwner::Agent("abc123".to_owned());

            let apply = |event, ctx: &mut AppContext| {
                BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                    state.apply_hotkey(window_id, &owner, event, ctx)
                })
            };
            assert_eq!(apply(HoldStart, ctx), Some(true));
            assert_eq!(apply(Toggle, ctx), Some(true));
            assert_eq!(apply(HoldEnd, ctx), Some(true));
        });
    });
}

#[test]
fn hotkeys_are_ignored_without_a_matching_underlay() {
    use warpui::platform::BrowserUnderlayHotkey::Toggle;

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);

            let result = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.apply_hotkey(
                    WindowId::new(),
                    &BrowserUnderlayOwner::Ambience,
                    Toggle,
                    ctx,
                )
            });
            assert_eq!(result, None);
        });
    });
}

#[test]
fn glass_opacity_setting_clamps_to_max() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);

            BrowserUnderlaySettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.glass_opacity.set_value(200, ctx).unwrap();
            });

            assert_eq!(*BrowserUnderlaySettings::as_ref(ctx).glass_opacity, 100);
        });
    });
}
