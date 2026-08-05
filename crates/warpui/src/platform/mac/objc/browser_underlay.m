#import "browser_underlay.h"

#import "host_view.h"

@implementation WarpBrowserUnderlayView

// While inert, the underlay is invisible to hit testing so all mouse events
// fall through to the host view exactly as if the underlay did not exist.
- (NSView *)hitTest:(NSPoint)point {
    return self.interactive ? [super hitTest:point] : nil;
}

- (BOOL)acceptsFirstResponder {
    return self.interactive;
}

@end

// Returns the container content view that can host an underlay, or nil. After
// the content-view restructure, WarpWindow's contentView is a plain container
// whose subviews include the WarpHostView; panels still use the host view as
// the content view directly and cannot host an underlay.
static NSView *underlay_container_for_window(NSWindow *window) {
    NSView *contentView = window.contentView;
    if (contentView == nil || [contentView isKindOfClass:[WarpHostView class]]) {
        return nil;
    }
    return contentView;
}

static NSURL *url_from_cstr(const char *url) {
    if (url == NULL) {
        return nil;
    }
    return [NSURL URLWithString:[NSString stringWithUTF8String:url]];
}

WarpBrowserUnderlayView *browser_underlay_for_window(NSWindow *window) {
    NSView *container = underlay_container_for_window(window);
    for (NSView *subview in container.subviews) {
        if ([subview isKindOfClass:[WarpBrowserUnderlayView class]]) {
            return (WarpBrowserUnderlayView *)subview;
        }
    }
    return nil;
}

BOOL browser_underlay_attach(NSWindow *window, const char *url) {
    NSView *container = underlay_container_for_window(window);
    if (container == nil) {
        return NO;
    }

    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    if (underlay == nil) {
        WKWebViewConfiguration *configuration = [[[WKWebViewConfiguration alloc] init] autorelease];
        // No persistent cookies/storage: the underlay is an ambient surface,
        // not a general-purpose browser profile.
        configuration.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
        // Ambient video backgrounds must start without a user gesture. Sound
        // still requires the page to be unmuted explicitly (interactive click
        // or JS), matching platform autoplay policies.
        configuration.mediaTypesRequiringUserActionForPlayback = WKAudiovisualMediaTypeNone;
        // Advertise the HTML5 element-fullscreen API (off by default in
        // WKWebView) so players show their fullscreen button at all…
        if (@available(macOS 12.3, *)) {
            configuration.preferences.elementFullscreenEnabled = YES;
        }
        // …but replace its implementation inside the page: WebKit presents
        // element fullscreen in a separate fullscreen Space, detaching the
        // video from the window. The shim below pins the requesting element
        // to the viewport instead — the underlay fills the window, so
        // "fullscreen" bleeds across the whole app background behind the
        // terminal glass — and mimics the fullscreen API surface
        // (fullscreenElement, fullscreenchange, the webkit-prefixed
        // variants) so players such as YouTube run their own fullscreen
        // layout and keep their controls working.
        NSString *fakeFullscreenShim = @""
            "(function () {\n"
            "  if (window.__warpFakeFullscreen) { return; }\n"
            "  window.__warpFakeFullscreen = true;\n"
            "  var fsElement = null;\n"
            "  var FILL = '__warp_fill_fullscreen';\n"
            "  function ensureStyle() {\n"
            "    if (document.getElementById('__warp_fill_style')) { return; }\n"
            "    var style = document.createElement('style');\n"
            "    style.id = '__warp_fill_style';\n"
            "    style.textContent = '.' + FILL + '{position:fixed !important;"
            "top:0 !important;left:0 !important;right:0 !important;bottom:0 !important;"
            "width:100vw !important;height:100vh !important;max-width:none !important;"
            "max-height:none !important;z-index:2147483647 !important;"
            "background:#000 !important;margin:0 !important;transform:none !important;}';\n"
            "    (document.head || document.documentElement).appendChild(style);\n"
            "  }\n"
            "  function fireChange() {\n"
            "    try { document.dispatchEvent(new Event('fullscreenchange')); } catch (e) {}\n"
            "    try { document.dispatchEvent(new Event('webkitfullscreenchange')); } catch (e) {}\n"
            "    try { window.dispatchEvent(new Event('resize')); } catch (e) {}\n"
            "  }\n"
            "  function enter(el) {\n"
            "    ensureStyle();\n"
            "    if (fsElement && fsElement !== el) { fsElement.classList.remove(FILL); }\n"
            "    fsElement = el;\n"
            "    el.classList.add(FILL);\n"
            "    fireChange();\n"
            "    return Promise.resolve();\n"
            "  }\n"
            "  function exit() {\n"
            "    if (fsElement) {\n"
            "      fsElement.classList.remove(FILL);\n"
            "      fsElement = null;\n"
            "      fireChange();\n"
            "    }\n"
            "    return Promise.resolve();\n"
            "  }\n"
            "  function defineGetter(proto, name, getter) {\n"
            "    try {\n"
            "      Object.defineProperty(proto, name, { configurable: true, get: getter });\n"
            "    } catch (e) {}\n"
            "  }\n"
            "  Element.prototype.requestFullscreen = function () { return enter(this); };\n"
            "  Element.prototype.webkitRequestFullscreen = function () { return enter(this); };\n"
            "  Element.prototype.webkitRequestFullScreen = function () { return enter(this); };\n"
            "  Document.prototype.exitFullscreen = function () { return exit(); };\n"
            "  Document.prototype.webkitExitFullscreen = function () { return exit(); };\n"
            "  Document.prototype.webkitCancelFullScreen = function () { return exit(); };\n"
            "  var current = function () { return fsElement; };\n"
            "  defineGetter(Document.prototype, 'fullscreenElement', current);\n"
            "  defineGetter(Document.prototype, 'webkitFullscreenElement', current);\n"
            "  defineGetter(Document.prototype, 'webkitCurrentFullScreenElement', current);\n"
            "  defineGetter(Document.prototype, 'webkitIsFullScreen', function () {\n"
            "    return fsElement != null;\n"
            "  });\n"
            "  defineGetter(Document.prototype, 'fullscreenEnabled', function () { return true; });\n"
            "  defineGetter(Document.prototype, 'webkitFullscreenEnabled', function () {\n"
            "    return true;\n"
            "  });\n"
            "  document.addEventListener('keydown', function (event) {\n"
            "    if (event.key === 'Escape' && fsElement != null) { exit(); }\n"
            "  }, true);\n"
            "})();";
        WKUserScript *fakeFullscreen =
            [[[WKUserScript alloc] initWithSource:fakeFullscreenShim
                                    injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                 forMainFrameOnly:NO] autorelease];
        [configuration.userContentController addUserScript:fakeFullscreen];
        // While inert, the page must not react to the pointer at all:
        // WebKit's mouse tracking follows the cursor irrespective of the
        // native hit-test gating, so hover UI (video controls, tooltips)
        // would appear under the terminal. Pages start with pointer-events
        // disabled; `browser_underlay_set_interactive` toggles it via
        // `__warpSetInteractive`. (A page navigated while interactive stays
        // inert until the next toggle — acceptable, the gesture re-runs it.)
        NSString *pointerGuardShim = @""
            "(function () {\n"
            "  if (window.__warpPointerGuard) { return; }\n"
            "  window.__warpPointerGuard = true;\n"
            "  window.__warpSetInteractive = function (on) {\n"
            "    try {\n"
            "      document.documentElement.style.pointerEvents = on ? '' : 'none';\n"
            "    } catch (e) {}\n"
            "  };\n"
            "  window.__warpSetInteractive(false);\n"
            "})();";
        WKUserScript *pointerGuard =
            [[[WKUserScript alloc] initWithSource:pointerGuardShim
                                    injectionTime:WKUserScriptInjectionTimeAtDocumentStart
                                 forMainFrameOnly:NO] autorelease];
        [configuration.userContentController addUserScript:pointerGuard];

        underlay = [[[WarpBrowserUnderlayView alloc] initWithFrame:container.bounds
                                                     configuration:configuration] autorelease];
        underlay.interactive = NO;
        underlay.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;

        WarpHostView *hostView = warp_host_view_for_window(window);
        [container addSubview:underlay positioned:NSWindowBelow relativeTo:hostView];
    }

    NSURL *requestUrl = url_from_cstr(url);
    if (requestUrl != nil) {
        [underlay loadRequest:[NSURLRequest requestWithURL:requestUrl]];
    }
    return YES;
}

void browser_underlay_navigate(NSWindow *window, const char *url) {
    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    NSURL *requestUrl = url_from_cstr(url);
    if (underlay != nil && requestUrl != nil) {
        [underlay loadRequest:[NSURLRequest requestWithURL:requestUrl]];
    }
}

// Serializes a JavaScript evaluation result to a string: strings pass through,
// everything else is JSON-encoded (numbers, booleans, arrays, objects, null).
static NSString *string_from_eval_result(id result) {
    if (result == nil || result == NSNull.null) {
        return @"null";
    }
    if ([result isKindOfClass:[NSString class]]) {
        return result;
    }
    NSError *error = nil;
    NSData *json = [NSJSONSerialization dataWithJSONObject:result
                                                   options:NSJSONWritingFragmentsAllowed
                                                     error:&error];
    if (json == nil) {
        return [result description];
    }
    return [[[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding] autorelease];
}

void browser_underlay_eval(NSWindow *window, const char *js, void *ctx,
                           BrowserUnderlayStringCallback callback) {
    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    if (underlay == nil || js == NULL) {
        callback(ctx, NULL, "no browser underlay attached to this window");
        return;
    }
    [underlay evaluateJavaScript:[NSString stringWithUTF8String:js]
               completionHandler:^(id result, NSError *error) {
                 if (error != nil) {
                     callback(ctx, NULL, error.localizedDescription.UTF8String);
                 } else {
                     callback(ctx, string_from_eval_result(result).UTF8String, NULL);
                 }
               }];
}

void browser_underlay_snapshot(NSWindow *window, void *ctx,
                               BrowserUnderlayDataCallback callback) {
    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    if (underlay == nil) {
        callback(ctx, NULL, 0, "no browser underlay attached to this window");
        return;
    }
    [underlay takeSnapshotWithConfiguration:nil
                          completionHandler:^(NSImage *image, NSError *error) {
                            if (image == nil) {
                                const char *message = error != nil
                                                          ? error.localizedDescription.UTF8String
                                                          : "snapshot returned no image";
                                callback(ctx, NULL, 0, message);
                                return;
                            }
                            NSBitmapImageRep *rep =
                                [NSBitmapImageRep imageRepWithData:image.TIFFRepresentation];
                            NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG
                                                            properties:@{}];
                            if (png == nil) {
                                callback(ctx, NULL, 0, "failed to encode snapshot as PNG");
                                return;
                            }
                            callback(ctx, png.bytes, png.length, NULL);
                          }];
}

void browser_underlay_set_interactive(NSWindow *window, BOOL interactive) {
    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    if (underlay == nil) {
        return;
    }
    // Order matters: `acceptsFirstResponder` consults this flag, so it must
    // be set before makeFirstResponder: can succeed.
    underlay.interactive = interactive;
    if (interactive) {
        // Hand the keyboard to the page — unless focus is already inside it:
        // re-asserting interactive (e.g. a hold ending while the latch is on)
        // must not blur the page's focused element.
        NSResponder *firstResponder = window.firstResponder;
        BOOL alreadyFocused = [firstResponder isKindOfClass:[NSView class]] &&
                              [(NSView *)firstResponder isDescendantOf:underlay];
        if (!alreadyFocused) {
            [window makeFirstResponder:underlay];
        }
    } else {
        // Return key focus to the host view so typing lands in the terminal.
        [window makeFirstResponder:warp_host_view_for_window(window)];
    }
    // Let the page see the pointer only while interactive (see the
    // pointer-guard user script installed at attach).
    NSString *pointerJs = interactive
                              ? @"window.__warpSetInteractive && window.__warpSetInteractive(true);"
                              : @"window.__warpSetInteractive && window.__warpSetInteractive(false);";
    [underlay evaluateJavaScript:pointerJs completionHandler:nil];
}

// --- Interactive-mode hotkeys ------------------------------------------------
//
// A single app-local NSEvent monitor recognizes the interactive-mode gestures.
// A local monitor sees events *before* they are dispatched to the first
// responder, which is what makes the gestures work while the webview owns key
// events. The monitor only recognizes gestures; the application flips
// interactive state in response to the callback, staying the single owner of
// that state.
//
// The gesture key is F18 — a key no physical keyboard emits — intended to be
// produced by remapping CapsLock to F18 at the OS level (hidutil/Karabiner):
//   tap F18            -> toggle latched interactive mode
//   hold F18 (>300ms)  -> interactive while held, back on release
//   Cmd+Esc            -> always return to the terminal
// A dedicated non-modifier key gives clean down/up pairs (unlike CapsLock or
// Globe/Fn themselves) and cannot collide with app keybindings.

static void *hotkey_ctx = NULL;
static BrowserUnderlayHotkeyCallback hotkey_callback = NULL;
static id hotkey_monitor = nil;

// F18 tap-vs-hold state. `hotkey_generation` invalidates the scheduled
// hold-activation when the key is released (a tap) before the threshold.
static BOOL f18_down = NO;
static BOOL f18_hold_active = NO;
static uint64_t hotkey_generation = 0;
static NSWindow *f18_window = nil; // retained while the key is down

static const unsigned short kKeyCodeEscape = 53;
static const unsigned short kKeyCodeF18 = 79;

// Held longer than this, F18 is a hold-to-interact; released sooner, a tap
// that toggles latched interactive mode.
static const int64_t kTapMaxMs = 300;

static void hotkey_clear_f18(void) {
    f18_down = NO;
    f18_hold_active = NO;
    hotkey_generation += 1;
    [f18_window release];
    f18_window = nil;
}

static NSEvent *hotkey_handle_event(NSEvent *event) {
    if (hotkey_callback == NULL) {
        return event;
    }

    if (event.type == NSEventTypeKeyDown && event.keyCode == kKeyCodeF18) {
        if (event.ARepeat || f18_down) {
            return nil; // swallow auto-repeat while held
        }
        NSWindow *window = event.window;
        if (window == nil || browser_underlay_for_window(window) == nil) {
            return event;
        }
        f18_down = YES;
        f18_window = [window retain];
        uint64_t generation = ++hotkey_generation;
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, kTapMaxMs * NSEC_PER_MSEC),
                       dispatch_get_main_queue(), ^{
                         if (f18_down && generation == hotkey_generation && f18_window != nil) {
                             f18_hold_active = YES;
                             hotkey_callback(hotkey_ctx, f18_window,
                                             BrowserUnderlayHotkeyHoldStart);
                         }
                       });
        return nil;
    }

    if (event.type == NSEventTypeKeyUp && event.keyCode == kKeyCodeF18) {
        if (!f18_down) {
            return event;
        }
        NSWindow *window = [[f18_window retain] autorelease];
        BOOL was_hold = f18_hold_active;
        hotkey_clear_f18();
        if (window != nil) {
            hotkey_callback(hotkey_ctx, window,
                            was_hold ? BrowserUnderlayHotkeyHoldEnd
                                     : BrowserUnderlayHotkeyToggle);
        }
        return nil;
    }

    if (event.type == NSEventTypeKeyDown && event.keyCode == kKeyCodeEscape) {
        NSWindow *window = event.window;
        WarpBrowserUnderlayView *underlay =
            window != nil ? browser_underlay_for_window(window) : nil;
        // Ignore a latched caps-lock state (possible before the CapsLock
        // remap, or from another keyboard) when matching the chord.
        NSEventModifierFlags mods =
            (event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask) &
            ~NSEventModifierFlagCapsLock;
        if (underlay != nil && underlay.interactive && mods == NSEventModifierFlagCommand) {
            hotkey_clear_f18();
            hotkey_callback(hotkey_ctx, window, BrowserUnderlayHotkeyForceOff);
            return nil;
        }
    }

    return event;
}

void browser_underlay_set_hotkey_callback(void *ctx, BrowserUnderlayHotkeyCallback callback) {
    hotkey_ctx = ctx;
    hotkey_callback = callback;
    if (hotkey_monitor == nil && callback != NULL) {
        hotkey_monitor = [[NSEvent
            addLocalMonitorForEventsMatchingMask:(NSEventMaskKeyDown | NSEventMaskKeyUp)
                                         handler:^NSEvent *(NSEvent *event) {
                                           return hotkey_handle_event(event);
                                         }] retain];
        // A key-up can be lost if the app deactivates while F18 is held; end
        // any in-flight gesture so the state cannot wedge (a stuck f18_down
        // would swallow every later F18 press).
        [[NSNotificationCenter defaultCenter]
            addObserverForName:NSApplicationDidResignActiveNotification
                        object:nil
                         queue:[NSOperationQueue mainQueue]
                    usingBlock:^(NSNotification *note) {
                      if (f18_hold_active && f18_window != nil && hotkey_callback != NULL) {
                          NSWindow *window = [[f18_window retain] autorelease];
                          hotkey_clear_f18();
                          hotkey_callback(hotkey_ctx, window, BrowserUnderlayHotkeyHoldEnd);
                      } else if (f18_down) {
                          hotkey_clear_f18();
                      }
                    }];
    }
}

void browser_underlay_detach(NSWindow *window) {
    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    if (underlay == nil) {
        return;
    }
    // Drop any in-flight hotkey gesture that belongs to this window.
    if (f18_window == window) {
        hotkey_clear_f18();
    }
    if (underlay.interactive) {
        browser_underlay_set_interactive(window, NO);
    }
    // Stop any playing media before the view is torn down.
    [underlay loadHTMLString:@"" baseURL:nil];
    [underlay removeFromSuperview];
}
