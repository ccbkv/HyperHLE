/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Legacy `UITransitionView`; nonzero transition types currently use a crossfade.

use super::UIViewHostObject;
use crate::frameworks::core_graphics::CGRect;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};
use crate::Environment;
use std::time::Instant;

#[derive(Default)]
struct TransitionHostObject {
    superclass: UIViewHostObject,
    delegate: id,
    from: id,
    to: id,
    timer: id,
    started: Option<Instant>,
    duration: f64,
    from_alpha: f32,
    to_alpha: f32,
    ignores_interaction: bool,
    previous_interaction: bool,
}
impl_HostObject_with_superclass!(TransitionHostObject);

fn finish(env: &mut Environment, view: id, completed: bool) {
    let (timer, from, to, delegate, interaction, from_alpha, to_alpha, had_transition) = {
        let host = env.objc.borrow_mut::<TransitionHostObject>(view);
        let had_transition = host.started.take().is_some();
        (
            std::mem::replace(&mut host.timer, nil),
            std::mem::replace(&mut host.from, nil),
            host.to,
            host.delegate,
            host.previous_interaction,
            host.from_alpha,
            host.to_alpha,
            had_transition,
        )
    };
    if !had_transition {
        return;
    }

    retain(env, view);
    if to != nil {
        retain(env, to);
    }
    if delegate != nil {
        retain(env, delegate);
    }

    if timer != nil {
        () = msg![env; timer invalidate];
        release(env, timer);
    }
    if from != nil {
        () = msg![env; from removeFromSuperview];
        () = msg![env; from setAlpha:from_alpha];
        release(env, from);
    }
    if to != nil {
        () = msg![env; to setAlpha:to_alpha];
    }
    () = msg![env; view setUserInteractionEnabled:interaction];

    let selector = env
        .objc
        .register_host_selector("transitionView:didComplete:".into(), &mut env.mem);
    if delegate != nil && msg![env; delegate respondsToSelector:selector] {
        () = msg![env; delegate transitionView:view didComplete:completed];
    }

    if delegate != nil {
        release(env, delegate);
    }
    if to != nil {
        release(env, to);
    }
    release(env, view);
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UITransitionView: UIView

+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<TransitionHostObject>::default(), &mut env.mem)
}

+ (f64)defaultDurationForTransition:(i32)transition {
    if transition == 0 { 0.0 } else { 0.3 }
}

- (id)delegate {
    env.objc.borrow::<TransitionHostObject>(this).delegate
}

- (())setDelegate:(id)delegate {
    env.objc.borrow_mut::<TransitionHostObject>(this).delegate = delegate;
}

- (id)fromView {
    env.objc.borrow::<TransitionHostObject>(this).from
}

- (id)toView {
    env.objc.borrow::<TransitionHostObject>(this).to
}

- (bool)isTransitioning {
    env.objc.borrow::<TransitionHostObject>(this).started.is_some()
}

- (bool)ignoresInteractionEvents {
    env.objc.borrow::<TransitionHostObject>(this).ignores_interaction
}

- (())setIgnoresInteractionEvents:(bool)ignores {
    env.objc.borrow_mut::<TransitionHostObject>(this).ignores_interaction = ignores;
}

- (f64)durationForTransition:(i32)transition {
    let delegate = env.objc.borrow::<TransitionHostObject>(this).delegate;
    let selector = env
        .objc
        .register_host_selector("durationForTransition:".into(), &mut env.mem);
    if delegate != nil && msg![env; delegate respondsToSelector:selector] {
        msg![env; delegate durationForTransition:transition]
    } else {
        msg_class![env; UITransitionView defaultDurationForTransition:transition]
    }
}

- (bool)transition:(i32)transition toView:(id)to {
    let mut from = env.objc.borrow::<TransitionHostObject>(this).to;
    if from == nil {
        from = env
            .objc
            .borrow::<UIViewHostObject>(this)
            .subviews
            .last()
            .copied()
            .unwrap_or(nil);
    }
    msg![env; this transition:transition fromView:from toView:to]
}

- (bool)transition:(i32)transition fromView:(id)from toView:(id)to {
    if to == nil || to == this || from == this {
        return false;
    }
    if env.objc.borrow::<TransitionHostObject>(this).started.is_some() {
        return false;
    }

    let mut ancestor: id = msg![env; this superview];
    while ancestor != nil {
        if ancestor == to || ancestor == from {
            return false;
        }
        ancestor = msg![env; ancestor superview];
    }

    retain(env, this);
    if from != nil {
        retain(env, from);
    }
    retain(env, to);

    let duration: f64 = msg![env; this durationForTransition:transition];
    let from_alpha: f32 = if from == nil {
        1.0
    } else {
        msg![env; from alpha]
    };
    let to_alpha: f32 = msg![env; to alpha];
    let previous_interaction: bool = msg![env; this isUserInteractionEnabled];
    let old_to = {
        let host = env.objc.borrow_mut::<TransitionHostObject>(this);
        host.from = from;
        host.from_alpha = from_alpha;
        host.to_alpha = to_alpha;
        host.previous_interaction = previous_interaction;
        host.duration = if duration.is_finite() { duration.max(0.0) } else { 0.3 };
        host.started = Some(Instant::now());
        std::mem::replace(&mut host.to, to)
    };

    if old_to != nil && old_to != from && old_to != to {
        () = msg![env; old_to removeFromSuperview];
    }
    if old_to != nil {
        release(env, old_to);
    }

    if env.objc.borrow::<TransitionHostObject>(this).ignores_interaction {
        () = msg![env; this setUserInteractionEnabled:false];
    }
    if from != nil && from != to {
        () = msg![env; this addSubview:from];
    }
    let bounds: CGRect = msg![env; this bounds];
    () = msg![env; to setFrame:bounds];
    () = msg![env; this addSubview:to];

    if from == to || transition == 0 || duration <= 0.0 {
        if from == to && from != nil {
            env.objc.borrow_mut::<TransitionHostObject>(this).from = nil;
            release(env, from);
        }
        finish(env, this, true);
    } else {
        () = msg![env; to setAlpha:0.0f32];
        let selector = env
            .objc
            .register_host_selector("_touchHLETransitionTick:".into(), &mut env.mem);
        let timer: id = msg_class![env; NSTimer
            scheduledTimerWithTimeInterval:(1.0f64 / 60.0)
            target:this selector:selector userInfo:nil repeats:true];
        retain(env, timer);
        env.objc.borrow_mut::<TransitionHostObject>(this).timer = timer;
    }

    release(env, this);
    true
}

- (())_touchHLETransitionTick:(id)timer {
    let (current_timer, started, duration, from, to, from_alpha, to_alpha) = {
        let host = env.objc.borrow::<TransitionHostObject>(this);
        (
            host.timer,
            host.started,
            host.duration,
            host.from,
            host.to,
            host.from_alpha,
            host.to_alpha,
        )
    };
    if current_timer != timer {
        return;
    }
    let Some(started) = started else {
        return;
    };

    let progress = if duration <= 0.0 {
        1.0
    } else {
        (started.elapsed().as_secs_f64() / duration).clamp(0.0, 1.0) as f32
    };

    if from != nil {
        retain(env, from);
    }
    retain(env, to);
    if from != nil {
        () = msg![env; from setAlpha:(from_alpha * (1.0 - progress))];
    }
    () = msg![env; to setAlpha:(to_alpha * progress)];
    if from != nil {
        release(env, from);
    }
    release(env, to);

    if progress >= 1.0 {
        finish(env, this, true);
    }
}

- (())dealloc {
    let (timer, from, to) = {
        let host = env.objc.borrow_mut::<TransitionHostObject>(this);
        (
            std::mem::replace(&mut host.timer, nil),
            std::mem::replace(&mut host.from, nil),
            std::mem::replace(&mut host.to, nil),
        )
    };
    if timer != nil {
        () = msg![env; timer invalidate];
        release(env, timer);
    }
    if from != nil {
        release(env, from);
    }
    if to != nil {
        release(env, to);
    }
    msg_super![env; this dealloc]
}

@end

};
