#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>

/// A WKWebView composited *behind* the Metal host view as an ambient window
/// background (e.g. a muted video stream, or a browser an agent is driving).
///
/// Inert by default: it never participates in hit testing or the responder
/// chain, so terminal input is unaffected. Setting `interactive` to YES lets
/// mouse and keyboard events reach the web content until it is reset.
@interface WarpBrowserUnderlayView : WKWebView
@property(nonatomic, assign) BOOL interactive;
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
    BrowserUnderlayHotkeyToggle = 0,    // toggle chord (Cmd+Shift+B)
    BrowserUnderlayHotkeyHoldStart = 1, // hold modifier (Globe/Fn) engaged
    BrowserUnderlayHotkeyHoldEnd = 2,   // hold modifier released
    BrowserUnderlayHotkeyForceOff = 3,  // escape chord (Cmd+Esc)
} BrowserUnderlayHotkeyEvent;

/// Callback invoked on the main thread when an interactive-mode hotkey fires
/// for a window with an attached underlay.
typedef void (*BrowserUnderlayHotkeyCallback)(void *ctx, NSWindow *window, int event);

// All functions must be called on the main thread. Functions taking a window
// no-op (or report an error through their callback) when the window has no
// underlay attached.

/// Attaches an underlay to `window` behind its Metal host view, creating it if
/// needed, and starts loading `url`. Returns NO if the window's view hierarchy
/// cannot host an underlay (e.g. panels, whose content view is the host view).
BOOL browser_underlay_attach(NSWindow *window, const char *url);

/// Returns the underlay attached to `window`, or nil.
WarpBrowserUnderlayView *browser_underlay_for_window(NSWindow *window);

void browser_underlay_navigate(NSWindow *window, const char *url);

void browser_underlay_eval(NSWindow *window, const char *js, void *ctx,
                           BrowserUnderlayStringCallback callback);

void browser_underlay_snapshot(NSWindow *window, void *ctx,
                               BrowserUnderlayDataCallback callback);

void browser_underlay_set_interactive(NSWindow *window, BOOL interactive);

void browser_underlay_detach(NSWindow *window);

/// Registers the process-wide interactive-mode hotkey callback (replacing any
/// previous registration) and installs an app-local event monitor. The monitor
/// only reacts to events in windows that have an underlay attached; hotkey
/// events are delivered through `callback` rather than acted on directly so
/// the application stays the single owner of interactive-mode state.
void browser_underlay_set_hotkey_callback(void *ctx, BrowserUnderlayHotkeyCallback callback);
