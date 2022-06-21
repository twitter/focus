// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::SessionStatus;
use std::time::Duration;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {}

// N.B. These were found in the core-foundation-rs crate.  Eventually we should consider adding `CGEventSourceSecondsSinceLastEventType` to that create and taking a dependency.  That code is dual-licensed under the Apache and MIT licenses.
// [Ref](https://github.com/servo/core-foundation-rs/tree/master/core-graphics/src)

/// Constants that specify the different types of input events.
///
/// [Ref](http://opensource.apple.com/source/IOHIDFamily/IOHIDFamily-700/IOHIDSystem/IOKit/hidsystem/IOLLEvent.h)
#[allow(dead_code)]
#[repr(u32)]
#[derive(Clone, Copy, Debug)]
enum CGEventType {
    Null = 0,

    // Mouse events.
    LeftMouseDown = 1,
    LeftMouseUp = 2,
    RightMouseDown = 3,
    RightMouseUp = 4,
    MouseMoved = 5,
    LeftMouseDragged = 6,
    RightMouseDragged = 7,

    // Keyboard events.
    KeyDown = 10,
    KeyUp = 11,
    FlagsChanged = 12,

    // Specialized control devices.
    ScrollWheel = 22,
    TabletPointer = 23,
    TabletProximity = 24,
    OtherMouseDown = 25,
    OtherMouseUp = 26,
    OtherMouseDragged = 27,

    // Out of band event types. These are delivered to the event tap callback
    // to notify it of unusual conditions that disable the event tap.
    TapDisabledByTimeout = 0xFFFFFFFE,
    TapDisabledByUserInput = 0xFFFFFFFF,
}

/// Constants specifying which input source state to query.
///
/// [Ref](https://developer.apple.com/documentation/coregraphics/cgeventsourcestateid?language=objc)
#[allow(dead_code)]
#[repr(i32)]
#[derive(Clone, Copy, Debug)]
enum CGEventSourceStateID {
    Private = -1,
    CombinedSessionState = 0,
    HIDSystemState = 1,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    // Return the duration in seconds since the last instance of the given event type.
    //
    // [Ref](https://developer.apple.com/documentation/coregraphics/1408790-cgeventsourcesecondssincelasteve/)
    fn CGEventSourceSecondsSinceLastEventType(
        event_source_state: CGEventSourceStateID,
        event_type: CGEventType,
    ) -> f64;
}

/// Returns whether the session has been idle (no keyboard or mouse input) for at least a given duration.
///
/// # Safety
///
/// This function uses native macOS Core Graphics APIs to determine whether the
#[cfg(target_os = "macos")]
pub unsafe fn has_session_been_idle_for(at_least: Duration) -> SessionStatus {
    let activity_threshold: f64 = at_least.as_secs_f64();

    let keyboard_events = vec![
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
    ];

    let mouse_events = vec![
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDragged,
        CGEventType::ScrollWheel,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::OtherMouseDragged,
    ];

    for event_type in keyboard_events.iter().chain(mouse_events.iter()) {
        let idle_time = CGEventSourceSecondsSinceLastEventType(
            CGEventSourceStateID::CombinedSessionState,
            *event_type,
        );
        if idle_time < activity_threshold {
            println!(
                "\t{:?} idle time {} is below threshold {}",
                event_type, idle_time, activity_threshold
            );
            return SessionStatus::Active;
        }
    }
    SessionStatus::Idle
}
