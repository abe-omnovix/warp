use settings::{PrivatePreferences, PublicPreferences, Setting as _, SettingsManager};
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

fn attach(window_id: WindowId, ctx: &mut AppContext) {
    BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
        state.set_attached(window_id, "https://example.com".to_owned(), ctx);
    });
}

#[test]
fn set_pane_glass_marks_pane_as_glass() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);

            let applied = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, None, ctx)
            });

            assert!(applied);
            let state = BrowserUnderlayState::as_ref(ctx);
            assert!(state.is_glass_pane(window_id, pane_id));
            assert!(state.has_glass_panes(window_id));
        });
    });
}

#[test]
fn set_pane_glass_is_noop_without_underlay() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();

            let applied = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, None, ctx)
            });

            assert!(!applied);
            assert!(!BrowserUnderlayState::as_ref(ctx).is_glass_pane(window_id, pane_id));
        });
    });
}

#[test]
fn disabling_glass_removes_pane_from_glass_set() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, None, ctx);
            });

            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, false, None, ctx);
            });

            let state = BrowserUnderlayState::as_ref(ctx);
            assert!(!state.is_glass_pane(window_id, pane_id));
            assert!(!state.has_glass_panes(window_id));
        });
    });
}

#[test]
fn detach_clears_glass_state() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, None, ctx);
            });

            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_detached(window_id, ctx);
            });

            let state = BrowserUnderlayState::as_ref(ctx);
            assert!(!state.is_attached(window_id));
            assert!(!state.has_glass_panes(window_id));
        });
    });
}

#[test]
fn pane_paints_no_background_in_ambience_mode() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);

            assert_eq!(pane_fill_opacity(window_id, pane_id, ctx), None);
        });
    });
}

#[test]
fn pane_paints_no_background_without_underlay() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);

            assert_eq!(
                pane_fill_opacity(WindowId::new(), test_pane_id(), ctx),
                None
            );
        });
    });
}

#[test]
fn glass_pane_uses_glass_opacity_setting() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, None, ctx);
            });

            // 55 is the `glass_opacity` setting default.
            assert_eq!(pane_fill_opacity(window_id, pane_id, ctx), Some(55));
        });
    });
}

#[test]
fn non_glass_pane_is_near_opaque_while_another_pane_is_glass() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let glass_pane = test_pane_id();
            let other_pane = test_pane_id();
            attach(window_id, ctx);
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, glass_pane, true, None, ctx);
            });

            assert_eq!(pane_fill_opacity(window_id, other_pane, ctx), Some(95));
        });
    });
}

#[test]
fn glass_opacity_override_beats_setting() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);

            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, Some(30), ctx);
            });

            assert_eq!(pane_fill_opacity(window_id, pane_id, ctx), Some(30));
        });
    });
}

#[test]
fn glass_opacity_override_clamps_to_max() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);

            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, Some(200), ctx);
            });

            assert_eq!(pane_fill_opacity(window_id, pane_id, ctx), Some(100));
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

#[test]
fn workspace_renders_whole_window_glass_without_glass_panes() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach(window_id, ctx);

            // 55 is the `glass_opacity` setting default.
            assert_eq!(workspace_fill_opacity(window_id, ctx), 55);
        });
    });
}

#[test]
fn workspace_drops_to_faint_tint_while_a_pane_is_glass() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            let pane_id = test_pane_id();
            attach(window_id, ctx);
            BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_pane_glass(window_id, pane_id, true, None, ctx);
            });

            assert_eq!(workspace_fill_opacity(window_id, ctx), 10);
        });
    });
}

fn apply_hotkey(
    window_id: WindowId,
    event: warpui::platform::BrowserUnderlayHotkey,
    ctx: &mut AppContext,
) -> Option<bool> {
    BrowserUnderlayState::handle(ctx)
        .update(ctx, |state, ctx| state.apply_hotkey(window_id, event, ctx))
}

#[test]
fn toggle_hotkey_flips_latched_interactive() {
    use warpui::platform::BrowserUnderlayHotkey::Toggle;

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach(window_id, ctx);

            assert_eq!(apply_hotkey(window_id, Toggle, ctx), Some(true));
            assert_eq!(apply_hotkey(window_id, Toggle, ctx), Some(false));
        });
    });
}

#[test]
fn hold_hotkey_is_momentary() {
    use warpui::platform::BrowserUnderlayHotkey::{HoldEnd, HoldStart};

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach(window_id, ctx);

            assert_eq!(apply_hotkey(window_id, HoldStart, ctx), Some(true));
            assert_eq!(apply_hotkey(window_id, HoldEnd, ctx), Some(false));
        });
    });
}

#[test]
fn toggle_during_hold_latches_interactive_past_release() {
    use warpui::platform::BrowserUnderlayHotkey::{HoldEnd, HoldStart, Toggle};

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach(window_id, ctx);

            assert_eq!(apply_hotkey(window_id, HoldStart, ctx), Some(true));
            assert_eq!(apply_hotkey(window_id, Toggle, ctx), Some(true));
            assert_eq!(apply_hotkey(window_id, HoldEnd, ctx), Some(true));
        });
    });
}

#[test]
fn force_off_clears_latched_and_hold() {
    use warpui::platform::BrowserUnderlayHotkey::{ForceOff, HoldStart, Toggle};

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach(window_id, ctx);
            apply_hotkey(window_id, Toggle, ctx);
            apply_hotkey(window_id, HoldStart, ctx);

            assert_eq!(apply_hotkey(window_id, ForceOff, ctx), Some(false));
        });
    });
}

#[test]
fn hotkeys_are_ignored_without_an_underlay() {
    use warpui::platform::BrowserUnderlayHotkey::Toggle;

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);

            assert_eq!(apply_hotkey(WindowId::new(), Toggle, ctx), None);
        });
    });
}

#[test]
fn latched_off_keeps_effective_interactive_while_held() {
    use warpui::platform::BrowserUnderlayHotkey::HoldStart;

    App::test((), |mut app| async move {
        app.update(|ctx| {
            init_test_app(ctx);
            let window_id = WindowId::new();
            attach(window_id, ctx);
            apply_hotkey(window_id, HoldStart, ctx);

            // Latching off over the control plane while the hold hotkey is
            // physically held: input keeps flowing until release.
            let effective = BrowserUnderlayState::handle(ctx).update(ctx, |state, ctx| {
                state.set_interactive(window_id, false, ctx)
            });

            assert_eq!(effective, Some(true));
        });
    });
}
