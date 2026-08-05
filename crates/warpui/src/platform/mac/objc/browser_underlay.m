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
    underlay.interactive = interactive;
    if (!interactive && underlay != nil) {
        // Return key focus to the host view so typing lands in the terminal.
        [window makeFirstResponder:warp_host_view_for_window(window)];
    }
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
        NSEventModifierFlags mods =
            event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
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
