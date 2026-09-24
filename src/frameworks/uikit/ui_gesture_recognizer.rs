/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Gesture recognizer interfaces, with local tap/swipe touch tracking.
//! Pan recognition and require-to-fail arbitration are not implemented.

use crate::frameworks::core_graphics::{CGFloat, CGPoint};
use crate::frameworks::foundation::{NSInteger, NSUInteger};
use crate::objc::{
    autorelease, id, msg, msg_class, msg_send, nil, objc_classes, release, retain, ClassExports, HostObject,
    NSZonePtr, SEL,
};
use crate::Environment;

pub type UIGestureRecognizerState = NSInteger;
pub const UIGestureRecognizerStatePossible: UIGestureRecognizerState = 0;
pub const UIGestureRecognizerStateBegan: UIGestureRecognizerState = 1;
pub const UIGestureRecognizerStateChanged: UIGestureRecognizerState = 2;
pub const UIGestureRecognizerStateEnded: UIGestureRecognizerState = 3;
pub const UIGestureRecognizerStateCancelled: UIGestureRecognizerState = 4;
pub const UIGestureRecognizerStateRecognized: UIGestureRecognizerState =
    UIGestureRecognizerStateEnded;
pub const UIGestureRecognizerStateFailed: UIGestureRecognizerState = 5;

pub type UISwipeGestureRecognizerDirection = NSUInteger;
pub const UISwipeGestureRecognizerDirectionRight: UISwipeGestureRecognizerDirection = 1 << 0;
pub const UISwipeGestureRecognizerDirectionLeft: UISwipeGestureRecognizerDirection = 1 << 1;
pub const UISwipeGestureRecognizerDirectionUp: UISwipeGestureRecognizerDirection = 1 << 2;
pub const UISwipeGestureRecognizerDirectionDown: UISwipeGestureRecognizerDirection = 1 << 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GestureKind {
    Generic,
    Tap,
    LongPress,
    Swipe,
}

pub(super) struct UIGestureRecognizerHostObject {
    // UIKit does not retain targets, delegates or the associated view.
    targets: Vec<(id, SEL)>,
    // Retained relationships, matching PR #95; not yet arbitrated.
    require_to_fail: Vec<id>,
    delegate: id,
    pub(super) view: id,
    kind: GestureKind,
    state: UIGestureRecognizerState,
    enabled: bool,
    cancels_touches_in_view: bool,
    delays_touches_began: bool,
    delays_touches_ended: bool,
    number_of_taps_required: NSUInteger,
    number_of_touches_required: NSUInteger,
    direction: UISwipeGestureRecognizerDirection,
    initial_location: CGPoint,
    current_location: CGPoint,
    tracking: bool,
    minimum_number_of_touches: NSUInteger,
    maximum_number_of_touches: NSUInteger,
    translation: CGPoint,
    velocity: CGPoint,
    minimum_press_duration: f64,
    allowable_movement: CGFloat,
    edges: NSUInteger,
    exclusive: bool,
    timer: id,
    tap_count: NSUInteger,
    last_tap_time: f64,
    last_tap_location: CGPoint,
}
impl HostObject for UIGestureRecognizerHostObject {}

impl UIGestureRecognizerHostObject {
    fn new(kind: GestureKind) -> Self {
        Self {
            targets: Vec::new(),
            require_to_fail: Vec::new(),
            delegate: nil,
            view: nil,
            kind,
            state: UIGestureRecognizerStatePossible,
            enabled: true,
            cancels_touches_in_view: true,
            delays_touches_began: false,
            delays_touches_ended: true,
            number_of_taps_required: 1,
            number_of_touches_required: 1,
            direction: UISwipeGestureRecognizerDirectionRight,
            initial_location: CGPoint { x: 0.0, y: 0.0 },
            current_location: CGPoint { x: 0.0, y: 0.0 },
            tracking: false,
            minimum_number_of_touches: 1,
            maximum_number_of_touches: NSInteger::MAX as NSUInteger,
            translation: CGPoint { x: 0.0, y: 0.0 },
            velocity: CGPoint { x: 0.0, y: 0.0 },
            minimum_press_duration: 0.5,
            allowable_movement: 10.0,
            edges: 0,
            exclusive: false,
            timer: nil,
            tap_count: 0,
            last_tap_time: f64::NEG_INFINITY,
            last_tap_location: CGPoint { x: 0.0, y: 0.0 },
        }
    }
}

impl Default for UIGestureRecognizerHostObject {
    fn default() -> Self {
        Self::new(GestureKind::Generic)
    }
}

fn init_with_target(env: &mut Environment, this: id, target: id, action: SEL) -> id {
    if target != nil && !action.is_null() {
        env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this)
            .targets.push((target, action));
    }
    this
}

fn stop_press_timer(env: &mut Environment, recognizer: id) {
    let timer = std::mem::replace(
        &mut env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer).timer,
        nil,
    );
    if timer != nil {
        () = msg![env; timer invalidate];
        release(env, timer);
    }
}

fn exclude_other_gestures(env: &mut Environment, recognizer: id) {
    let host = env.objc.borrow::<UIGestureRecognizerHostObject>(recognizer);
    if !host.exclusive || host.view == nil { return; }
    let others = super::ui_view::gesture_recognizers(env, host.view);
    for &other in &others { retain(env, other); }
    for other in others {
        if other != recognizer {
            stop_press_timer(env, other);
            let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(other);
            host.tracking = false;
            host.tap_count = 0;
            host.state = UIGestureRecognizerStateCancelled;
        }
        release(env, other);
    }
}

pub fn fire_targets(env: &mut Environment, recognizer: id) {
    let targets = env.objc.borrow::<UIGestureRecognizerHostObject>(recognizer)
        .targets.clone();
    retain(env, recognizer);
    for (target, action) in targets {
        if target == nil || action.is_null() {
            continue;
        }
        let colon_count = action.as_str(&env.mem).bytes()
            .filter(|&b| b == b':').count();
        match colon_count {
            0 => () = msg_send(env, (target, action)),
            1 => () = msg_send(env, (target, action, recognizer)),
            _ => log!("Unexpected gesture recognizer action {:?}", action),
        }
    }
    release(env, recognizer);
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIGestureRecognizer: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UIGestureRecognizerHostObject::new(GestureKind::Generic));
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithTarget:(id)target action:(SEL)action {
    init_with_target(env, this, target, action)
}

- (())dealloc {
    let required = std::mem::take(
        &mut env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).require_to_fail,
    );
    for other in required {
        release(env, other);
    }
    env.objc.dealloc_object(this, &mut env.mem)
}

- (())addTarget:(id)target action:(SEL)action {
    init_with_target(env, this, target, action);
}

- (())removeTarget:(id)target action:(SEL)action {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).targets
        .retain(|&(t, a)| (target != nil && t != target)
            || (!action.is_null() && a != action));
}

- (())requireGestureRecognizerToFail:(id)other {
    if other == nil || other == this {
        return;
    }
    if env.objc.borrow::<UIGestureRecognizerHostObject>(this)
        .require_to_fail.contains(&other) {
        return;
    }
    retain(env, other);
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this)
        .require_to_fail.push(other);
}

- (())setState:(UIGestureRecognizerState)state {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).state = state;
}

- (())reset {
    stop_press_timer(env, this);
    let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this);
    host.tap_count = 0;
    host.state = UIGestureRecognizerStatePossible;
    host.tracking = false;
}

- (NSUInteger)numberOfTouches {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).tracking as NSUInteger
}

- (CGPoint)locationOfTouch:(NSUInteger)index inView:(id)view {
    let tracking = env.objc.borrow::<UIGestureRecognizerHostObject>(this).tracking;
    if index == 0 && tracking {
        msg![env; this locationInView:view]
    } else {
        CGPoint { x: 0.0, y: 0.0 }
    }
}

- (())touchesBegan:(id)_touches withEvent:(id)_event {}
- (())touchesMoved:(id)_touches withEvent:(id)_event {}
- (())touchesEnded:(id)_touches withEvent:(id)_event {}
- (())touchesCancelled:(id)_touches withEvent:(id)_event {
    stop_press_timer(env, this);
    let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this);
    host.tap_count = 0;
    host.tracking = false;
    host.state = UIGestureRecognizerStateCancelled;
}

- (id)delegate { env.objc.borrow::<UIGestureRecognizerHostObject>(this).delegate }
- (())setDelegate:(id)delegate {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).delegate = delegate;
}
- (id)view { env.objc.borrow::<UIGestureRecognizerHostObject>(this).view }
- (())setView:(id)view {
    stop_press_timer(env, this);
    let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this);
    host.tracking = false;
    host.tap_count = 0;
    host.view = view;
}
- (bool)isExclusive {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).exclusive
}
- (())setExclusive:(bool)exclusive {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).exclusive = exclusive;
}
- (UIGestureRecognizerState)state {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).state
}
- (bool)isEnabled { env.objc.borrow::<UIGestureRecognizerHostObject>(this).enabled }
- (())setEnabled:(bool)enabled {
    stop_press_timer(env, this);
    let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this);
    host.tap_count = 0;
    host.enabled = enabled;
    host.tracking = false;
    host.state = UIGestureRecognizerStatePossible;
}
- (bool)cancelsTouchesInView {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).cancels_touches_in_view
}
- (())setCancelsTouchesInView:(bool)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).cancels_touches_in_view = value;
}
- (bool)delaysTouchesBegan {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).delays_touches_began
}
- (())setDelaysTouchesBegan:(bool)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).delays_touches_began = value;
}
- (bool)delaysTouchesEnded {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).delays_touches_ended
}
- (())setDelaysTouchesEnded:(bool)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).delays_touches_ended = value;
}
- (CGPoint)locationInView:(id)view {
    let (current_location, own_view) = {
        let host = env.objc.borrow::<UIGestureRecognizerHostObject>(this);
        (host.current_location, host.view)
    };
    if view == nil || view == own_view {
        current_location
    } else {
        msg![env; view convertPoint:current_location fromView:own_view]
    }
}

@end

@implementation UIPanGestureRecognizer: UIGestureRecognizer

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::<UIGestureRecognizerHostObject>::default();
    env.objc.alloc_object(this, host, &mut env.mem)
}

- (NSUInteger)minimumNumberOfTouches {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).minimum_number_of_touches
}
- (())setMinimumNumberOfTouches:(NSUInteger)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).minimum_number_of_touches = value;
}
- (NSUInteger)maximumNumberOfTouches {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).maximum_number_of_touches
}
- (())setMaximumNumberOfTouches:(NSUInteger)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).maximum_number_of_touches = value;
}
- (CGPoint)translationInView:(id)_view {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).translation
}
- (())setTranslation:(CGPoint)translation inView:(id)_view {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).translation = translation;
}
- (CGPoint)velocityInView:(id)_view {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).velocity
}

@end

@implementation UIScreenEdgePanGestureRecognizer: UIPanGestureRecognizer

- (NSUInteger)edges {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).edges
}
- (())setEdges:(NSUInteger)edges {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).edges = edges;
}

@end

@implementation UITapGestureRecognizer: UIGestureRecognizer

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UIGestureRecognizerHostObject::new(GestureKind::Tap));
    env.objc.alloc_object(this, host_object, &mut env.mem)
}
- (NSUInteger)numberOfTapsRequired {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_taps_required
}
- (())setNumberOfTapsRequired:(NSUInteger)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_taps_required = value;
}
- (NSUInteger)numberOfTouchesRequired {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_touches_required
}
- (())setNumberOfTouchesRequired:(NSUInteger)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_touches_required = value;
}

@end


@implementation UITextTapRecognizer: UIGestureRecognizer

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::new(UIGestureRecognizerHostObject::new(GestureKind::Tap));
    env.objc.alloc_object(this, host, &mut env.mem)
}
- (i32)numberOfFingers {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_touches_required as i32
}
- (())setNumberOfFingers:(i32)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_touches_required = value.max(1) as NSUInteger;
}
- (i32)numberOfTaps {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_taps_required as i32
}
- (())setNumberOfTaps:(i32)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_taps_required = value.max(1) as NSUInteger;
}
- (CGFloat)allowableMovement {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).allowable_movement
}
- (())setAllowableMovement:(CGFloat)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).allowable_movement = value;
}
- (CGPoint)location {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).current_location
}
- (CGPoint)centroid { msg![env; this location] }

@end

@implementation UILongPressGestureRecognizer: UIGestureRecognizer

+ (id)allocWithZone:(NSZonePtr)_zone {
    let mut host = Box::new(UIGestureRecognizerHostObject::new(GestureKind::LongPress));
    host.number_of_taps_required = 0;
    env.objc.alloc_object(this, host, &mut env.mem)
}

- (i32)numberOfFingers {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_touches_required as i32
}
- (())setNumberOfFingers:(i32)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_touches_required = value.max(1) as NSUInteger;
}
- (f64)delay { msg![env; this minimumPressDuration] }
- (())setDelay:(f64)value { () = msg![env; this setMinimumPressDuration:value]; }

- (())_touchHLEPressTimer:(id)timer {
    let host = env.objc.borrow::<UIGestureRecognizerHostObject>(this);
    if host.timer != timer { return; }
    let active = host.enabled && host.tracking && host.view != nil
        && host.state == UIGestureRecognizerStatePossible;
    retain(env, this);
    stop_press_timer(env, this);
    if active && delegate_allows_begin(env, this) {
        env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).state = UIGestureRecognizerStateBegan;
        exclude_other_gestures(env, this);
        fire_targets(env, this);
    }
    release(env, this);
}

- (f64)minimumPressDuration {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).minimum_press_duration
}
- (())setMinimumPressDuration:(f64)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).minimum_press_duration = value;
}
- (CGFloat)allowableMovement {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).allowable_movement
}
- (())setAllowableMovement:(CGFloat)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).allowable_movement = value;
}
- (NSUInteger)numberOfTapsRequired {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_taps_required
}
- (())setNumberOfTapsRequired:(NSUInteger)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_taps_required = value;
}
- (NSUInteger)numberOfTouchesRequired {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_touches_required
}
- (())setNumberOfTouchesRequired:(NSUInteger)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_touches_required = value;
}

@end

@implementation UISwipeGestureRecognizer: UIGestureRecognizer

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UIGestureRecognizerHostObject::new(GestureKind::Swipe));
    env.objc.alloc_object(this, host_object, &mut env.mem)
}
- (UISwipeGestureRecognizerDirection)direction {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).direction
}
- (())setDirection:(UISwipeGestureRecognizerDirection)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).direction = value;
}
- (NSUInteger)numberOfTouchesRequired {
    env.objc.borrow::<UIGestureRecognizerHostObject>(this).number_of_touches_required
}
- (())setNumberOfTouchesRequired:(NSUInteger)value {
    env.objc.borrow_mut::<UIGestureRecognizerHostObject>(this).number_of_touches_required = value;
}

@end

};

pub(super) fn set_view(env: &mut Environment, recognizer: id, view: id) {
    () = msg![env; recognizer setView:view];
}

fn delegate_allows_touch(env: &mut Environment, recognizer: id, touch: id) -> bool {
    let delegate = env
        .objc
        .borrow::<UIGestureRecognizerHostObject>(recognizer)
        .delegate;
    if delegate == nil {
        return true;
    }
    let Some(selector) = env
        .objc
        .lookup_selector("gestureRecognizer:shouldReceiveTouch:")
    else {
        return true;
    };
    if msg![env; delegate respondsToSelector:selector] {
        msg_send(env, (delegate, selector, recognizer, touch))
    } else {
        true
    }
}

fn delegate_allows_begin(env: &mut Environment, recognizer: id) -> bool {
    let delegate = env
        .objc
        .borrow::<UIGestureRecognizerHostObject>(recognizer)
        .delegate;
    if delegate == nil {
        return true;
    }
    let Some(selector) = env.objc.lookup_selector("gestureRecognizerShouldBegin:") else {
        return true;
    };
    if msg![env; delegate respondsToSelector:selector] {
        msg_send(env, (delegate, selector, recognizer))
    } else {
        true
    }
}

pub(super) fn touches_began(env: &mut Environment, view: id, touches: id) {
    if view == nil {
        return;
    }
    let touch: id = msg![env; touches anyObject];
    if touch == nil {
        return;
    }
    let recognizers = super::ui_view::gesture_recognizers(env, view);
    for &recognizer in &recognizers { retain(env, recognizer); autorelease(env, recognizer); }
    for recognizer in recognizers {
        if env.objc.borrow::<UIGestureRecognizerHostObject>(recognizer).tracking {
            stop_press_timer(env, recognizer);
            let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer);
            host.tracking = false;
            host.tap_count = 0;
            host.state = UIGestureRecognizerStateFailed;
            continue;
        }
        let (enabled, recognizer_view) = {
            let host = env.objc.borrow::<UIGestureRecognizerHostObject>(recognizer);
            (host.enabled, host.view)
        };
        if !enabled || !delegate_allows_touch(env, recognizer, touch) {
            continue;
        }
        let location: CGPoint = msg![env; touch locationInView:recognizer_view];
        let host = env
            .objc
            .borrow_mut::<UIGestureRecognizerHostObject>(recognizer);
        host.initial_location = location;
        host.current_location = location;
        host.state = UIGestureRecognizerStatePossible;
        host.tracking = true;
        let kind = host.kind;
        let duration = host.minimum_press_duration;
        let supported = host.number_of_touches_required == 1;
        let count: NSUInteger = msg![env; touches count];
        stop_press_timer(env, recognizer);
        if !supported || count != 1 {
            let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer);
            host.tracking = false;
            host.state = UIGestureRecognizerStateFailed;
        } else if kind == GestureKind::LongPress {
            let duration = if duration.is_finite() { duration.max(0.001) } else { 0.5 };
            let selector = env.objc.register_host_selector("_touchHLEPressTimer:".into(), &mut env.mem);
            let timer: id = msg_class![env; NSTimer scheduledTimerWithTimeInterval:duration
                target:recognizer selector:selector userInfo:nil repeats:false];
            retain(env, timer);
            env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer).timer = timer;
        }
    }
}

pub(super) fn touches_moved(env: &mut Environment, view: id, touches: id) {
    if view == nil {
        return;
    }
    let touch: id = msg![env; touches anyObject];
    if touch == nil {
        return;
    }
    let recognizers = super::ui_view::gesture_recognizers(env, view);
    for &recognizer in &recognizers { retain(env, recognizer); autorelease(env, recognizer); }
    for recognizer in recognizers {
        let (tracking, recognizer_view) = {
            let host = env.objc.borrow::<UIGestureRecognizerHostObject>(recognizer);
            (host.tracking, host.view)
        };
        if tracking {
            let location: CGPoint = msg![env; touch locationInView:recognizer_view];
            let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer);
            host.current_location = location;
            let dx = location.x - host.initial_location.x;
            let dy = location.y - host.initial_location.y;
            let long_press = host.kind == GestureKind::LongPress;
            let active = long_press && matches!(host.state,
                UIGestureRecognizerStateBegan | UIGestureRecognizerStateChanged);
            let moved_too_far = dx * dx + dy * dy > host.allowable_movement.powi(2);
            if active {
                host.state = UIGestureRecognizerStateChanged;
                fire_targets(env, recognizer);
            } else if moved_too_far && matches!(host.kind, GestureKind::Tap | GestureKind::LongPress) {
                host.tracking = false;
                host.tap_count = 0;
                host.state = UIGestureRecognizerStateFailed;
                stop_press_timer(env, recognizer);
            }
        }
    }
}

pub(super) fn touches_ended(env: &mut Environment, view: id, touches: id) {
    if view == nil {
        return;
    }
    let touch: id = msg![env; touches anyObject];
    if touch == nil {
        return;
    }
    let recognizers = super::ui_view::gesture_recognizers(env, view);
    for &recognizer in &recognizers { retain(env, recognizer); autorelease(env, recognizer); }
    for recognizer in recognizers {
        let (tracking, recognizer_view) = {
            let host = env.objc.borrow::<UIGestureRecognizerHostObject>(recognizer);
            (host.tracking, host.view)
        };
        if !tracking {
            continue;
        }
        let location: CGPoint = msg![env; touch locationInView:recognizer_view];
        {
            let host = env
                .objc
                .borrow_mut::<UIGestureRecognizerHostObject>(recognizer);
            host.current_location = location;
            host.tracking = false;
        }

        stop_press_timer(env, recognizer);
        let host = env.objc.borrow::<UIGestureRecognizerHostObject>(recognizer);
        if host.kind == GestureKind::LongPress {
            let active = matches!(host.state, UIGestureRecognizerStateBegan | UIGestureRecognizerStateChanged);
            env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer).state =
                if active { UIGestureRecognizerStateEnded } else { UIGestureRecognizerStateFailed };
            if active { fire_targets(env, recognizer); }
            continue;
        }
        let timestamp: f64 = msg![env; touch timestamp];
        let recognized = {
            let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer);
            let dx = host.current_location.x - host.initial_location.x;
            let dy = host.current_location.y - host.initial_location.y;
            match host.kind {
                GestureKind::Tap => {
                    let distance = host.allowable_movement.powi(2);
                    let lx = location.x - host.last_tap_location.x;
                    let ly = location.y - host.last_tap_location.y;
                    let elapsed = timestamp - host.last_tap_time;
                    if elapsed < 0.0 || elapsed > 0.35 || lx * lx + ly * ly > distance {
                        host.tap_count = 0;
                    }
                    if dx * dx + dy * dy <= distance {
                        host.tap_count = host.tap_count.saturating_add(1);
                        host.last_tap_time = timestamp;
                        host.last_tap_location = location;
                        host.tap_count == host.number_of_taps_required
                    } else { host.tap_count = 0; false }
                }
                GestureKind::Swipe => {
                    let (direction, distance, cross_distance) = if dx.abs() >= dy.abs() {
                        (
                            if dx >= 0.0 {
                                UISwipeGestureRecognizerDirectionRight
                            } else {
                                UISwipeGestureRecognizerDirectionLeft
                            },
                            dx.abs(),
                            dy.abs(),
                        )
                    } else {
                        (
                            if dy >= 0.0 {
                                UISwipeGestureRecognizerDirectionDown
                            } else {
                                UISwipeGestureRecognizerDirectionUp
                            },
                            dy.abs(),
                            dx.abs(),
                        )
                    };
                    distance >= 24.0
                        && cross_distance <= distance
                        && host.direction & direction != 0
                }
                GestureKind::Generic | GestureKind::LongPress => false,
            }
        };

        if recognized && delegate_allows_begin(env, recognizer) {
            let host = env.objc.borrow_mut::<UIGestureRecognizerHostObject>(recognizer);
            host.state = UIGestureRecognizerStateRecognized;
            host.tap_count = 0;
            exclude_other_gestures(env, recognizer);
            fire_targets(env, recognizer);
        } else {
            env.objc
                .borrow_mut::<UIGestureRecognizerHostObject>(recognizer)
                .state = UIGestureRecognizerStateFailed;
        }
    }
}
