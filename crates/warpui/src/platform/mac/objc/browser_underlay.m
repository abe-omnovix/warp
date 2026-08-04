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

void browser_underlay_detach(NSWindow *window) {
    WarpBrowserUnderlayView *underlay = browser_underlay_for_window(window);
    if (underlay == nil) {
        return;
    }
    if (underlay.interactive) {
        browser_underlay_set_interactive(window, NO);
    }
    // Stop any playing media before the view is torn down.
    [underlay loadHTMLString:@"" baseURL:nil];
    [underlay removeFromSuperview];
}
