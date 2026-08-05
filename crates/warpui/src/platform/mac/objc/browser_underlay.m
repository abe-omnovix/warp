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
// responder, which is what makes the exit chord work while the webview owns
// key events. The monitor only recognizes gestures; the application flips
// interactive state in response to the callback, staying the single owner of
// that state.

static void *hotkey_ctx = NULL;
static BrowserUnderlayHotkeyCallback hotkey_callback = NULL;
static id hotkey_monitor = nil;

// Momentary-hold state. `hold_pending` is set between the hold modifier going
// down and the grace period elapsing; `hold_generation` invalidates a pending
// activation when the modifier is released (or a combo key is pressed) before
// the grace period fires.
static BOOL hold_pending = NO;
static BOOL hold_active = NO;
static uint64_t hold_generation = 0;
static NSWindow *hold_window = nil; // retained while pending or active

static const unsigned short kKeyCodeB = 11;
static const unsigned short kKeyCodeEscape = 53;
static const unsigned short kKeyCodeFunction = 63;

// Delay before a held Globe/Fn key becomes "interactive while held". Pressing
// any key inside the grace period cancels activation, so Fn-combos (fn+arrows
// for paging, fn-function-keys) keep going to the terminal.
static const int64_t kHoldGraceMs = 150;

static void hotkey_set_hold_window(NSWindow *window) {
    if (hold_window != window) {
        [hold_window release];
        hold_window = [window retain];
    }
}

static void hotkey_clear_hold(void) {
    hold_pending = NO;
    hold_generation += 1;
    [hold_window release];
    hold_window = nil;
}

static NSEvent *hotkey_handle_event(NSEvent *event) {
    if (hotkey_callback == NULL) {
        return event;
    }

    if (event.type == NSEventTypeFlagsChanged) {
        if (event.keyCode != kKeyCodeFunction) {
            return event;
        }
        BOOL down = (event.modifierFlags & NSEventModifierFlagFunction) != 0;
        if (down) {
            NSWindow *window = event.window;
            if (!hold_pending && !hold_active && window != nil &&
                browser_underlay_for_window(window) != nil) {
                hold_pending = YES;
                hotkey_set_hold_window(window);
                uint64_t generation = ++hold_generation;
                dispatch_after(
                    dispatch_time(DISPATCH_TIME_NOW, kHoldGraceMs * NSEC_PER_MSEC),
                    dispatch_get_main_queue(), ^{
                      if (hold_pending && generation == hold_generation && hold_window != nil) {
                          hold_pending = NO;
                          hold_active = YES;
                          hotkey_callback(hotkey_ctx, hold_window, BrowserUnderlayHotkeyHoldStart);
                      }
                    });
            }
        } else {
            if (hold_active) {
                NSWindow *window = [[hold_window retain] autorelease];
                hold_active = NO;
                hotkey_clear_hold();
                hotkey_callback(hotkey_ctx, window, BrowserUnderlayHotkeyHoldEnd);
            } else if (hold_pending) {
                hotkey_clear_hold();
            }
        }
        // Modifier transitions always continue to normal dispatch.
        return event;
    }

    if (event.type != NSEventTypeKeyDown) {
        return event;
    }

    // Any key pressed during the grace period means the user is typing an
    // Fn-combo, not holding to interact.
    if (hold_pending) {
        hotkey_clear_hold();
    }

    NSWindow *window = event.window;
    if (window == nil) {
        return event;
    }
    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    if (underlay == nil) {
        return event;
    }

    NSEventModifierFlags mods =
        event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
    if (event.keyCode == kKeyCodeB &&
        mods == (NSEventModifierFlagCommand | NSEventModifierFlagShift)) {
        hotkey_callback(hotkey_ctx, window, BrowserUnderlayHotkeyToggle);
        return nil;
    }
    if (underlay.interactive && event.keyCode == kKeyCodeEscape &&
        mods == NSEventModifierFlagCommand) {
        if (hold_active) {
            hold_active = NO;
            hotkey_clear_hold();
        }
        hotkey_callback(hotkey_ctx, window, BrowserUnderlayHotkeyForceOff);
        return nil;
    }
    return event;
}

void browser_underlay_set_hotkey_callback(void *ctx, BrowserUnderlayHotkeyCallback callback) {
    hotkey_ctx = ctx;
    hotkey_callback = callback;
    if (hotkey_monitor == nil && callback != NULL) {
        hotkey_monitor = [[NSEvent
            addLocalMonitorForEventsMatchingMask:(NSEventMaskKeyDown | NSEventMaskFlagsChanged)
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
    // Drop any in-flight hold gesture that belongs to this window.
    if (hold_window == window) {
        hold_active = NO;
        hotkey_clear_hold();
    }
    if (underlay.interactive) {
        browser_underlay_set_interactive(window, NO);
    }
    // Stop any playing media before the view is torn down.
    [underlay loadHTMLString:@"" baseURL:nil];
    [underlay removeFromSuperview];
}
