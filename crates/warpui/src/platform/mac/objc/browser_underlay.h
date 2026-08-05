#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>

/// A WKWebView composited *behind* the Metal host view as an ambient window
/// background (e.g. a muted video stream, or a browser an agent is driving).
///
/// A window can host several underlays, distinguished by `owner`:
/// - the empty string: the window's ambience underlay (user-owned, always at
///   the very back);
/// - anything else: an agent-owned underlay keyed by the agent's pane (in
///   front of the ambience, behind the Metal surface). At most one underlay
///   is unhidden at a time above the ambience; the application drives
///   visibility so an agent underlay only shows while its pane's tab is
///   frontmost.
///
/// Inert by default: underlays never participate in hit testing or the
/// responder chain, so terminal input is unaffected. Setting `interactive` to
/// YES lets mouse and keyboard events reach the *visible* underlay until it
/// is reset.
@interface WarpBrowserUnderlayView : WKWebView
@property(nonatomic, assign) BOOL interactive;
/// Identity of this underlay within its window ("" = ambience).
@property(nonatomic, copy) NSString *owner;
@end

/// Callback receiving a JavaScript evaluation result. Exactly one of `result`
/// and `error` is non-NULL. The strings are only valid for the duration of the
/// call. Always invoked on the main thread.
typedef void (*BrowserUnderlayStringCallback)(void *ctx, const char *result, const char *error);

/// Callback receiving PNG-encoded snapshot bytes. On failure `bytes` is NULL
/// and `error` describes the failure. The buffer is only valid for the
/// duration of the call. Always invoked on the main thread.
typedef void (*BrowserUnderlayDataCallback)(void *ctx, const uint8_t *bytes, size_t len,
                                            const char *error);

/// Interactive-mode hotkey gestures. Values match the Rust-side
/// `BrowserUnderlayHotkey` enum in `warpui_core::platform`.
typedef enum {
    BrowserUnderlayHotkeyToggle = 0,    // toggle chord (tap CapsLock-as-F18)
    BrowserUnderlayHotkeyHoldStart = 1, // hold key engaged
    BrowserUnderlayHotkeyHoldEnd = 2,   // hold key released
    BrowserUnderlayHotkeyForceOff = 3,  // escape chord (Cmd+Esc)
} BrowserUnderlayHotkeyEvent;

/// Callback invoked on the main thread when an interactive-mode hotkey fires
/// for a window's visible underlay. `owner` identifies that underlay ("" =
/// ambience) and is only valid for the duration of the call.
typedef void (*BrowserUnderlayHotkeyCallback)(void *ctx, NSWindow *window, const char *owner,
                                              int event);

// All functions must be called on the main thread. `owner` is the underlay
// identity within the window; NULL and "" both select the ambience underlay.
// Functions taking a window no-op (or report an error through their callback)
// when the window has no underlay with that owner.

/// Attaches the `owner` underlay to `window` behind its Metal host view,
/// creating it if needed, and starts loading `url`. Agent underlays are
/// created hidden; the application unhides them via
/// `browser_underlay_set_visible` when their pane's tab is frontmost.
/// Returns NO if the window's view hierarchy cannot host an underlay
/// (e.g. panels, whose content view is the host view).
BOOL browser_underlay_attach(NSWindow *window, const char *owner, const char *url);

/// Returns the `owner` underlay attached to `window`, or nil.
WarpBrowserUnderlayView *browser_underlay_for_window(NSWindow *window, const char *owner);

/// Returns the frontmost unhidden underlay of `window`, or nil. This is the
/// underlay the user sees (and the one interactive mode applies to).
WarpBrowserUnderlayView *browser_underlay_visible_for_window(NSWindow *window);

void browser_underlay_navigate(NSWindow *window, const char *owner, const char *url);

void browser_underlay_eval(NSWindow *window, const char *owner, const char *js, void *ctx,
                           BrowserUnderlayStringCallback callback);

void browser_underlay_snapshot(NSWindow *window, const char *owner, void *ctx,
                               BrowserUnderlayDataCallback callback);

void browser_underlay_set_interactive(NSWindow *window, const char *owner, BOOL interactive);

/// Shows or hides the `owner` underlay. Hiding an interactive underlay also
/// clears its interactive state and returns key focus to the host view.
void browser_underlay_set_visible(NSWindow *window, const char *owner, BOOL visible);

void browser_underlay_detach(NSWindow *window, const char *owner);

/// Registers the process-wide interactive-mode hotkey callback (replacing any
/// previous registration) and installs an app-local event monitor. The monitor
/// only reacts to events in windows that have a visible underlay; hotkey
/// events are delivered through `callback` rather than acted on directly so
/// the application stays the single owner of interactive-mode state.
void browser_underlay_set_hotkey_callback(void *ctx, BrowserUnderlayHotkeyCallback callback);
