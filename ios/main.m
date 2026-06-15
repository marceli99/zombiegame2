// iOS entry point.  `#[bevy_main]` in the Rust lib exports a C-callable
// `main_rs` (for `target_os = "ios"`); this Objective-C `main` just calls it.
// Bevy/winit then take over the UIApplication run loop from inside Rust.
#import <UIKit/UIKit.h>

extern void main_rs(void);

int main(int argc, char *argv[]) {
    @autoreleasepool {
        main_rs();
    }
    return 0;
}
