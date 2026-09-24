/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Minimal `GraphicsServices.framework` compatibility for legacy UIKit callers.

use crate::dyld::{export_c_func, FunctionExports, HostDylib};
use crate::Environment;

fn GSStatusBarHeight(_env: &mut Environment, _mode: i32, _orientation: i32) -> f32 {
    // touchHLE does not draw a system status bar, so callers should not reserve
    // any extra vertical space for it.
    0.0
}

pub const FUNCTIONS: FunctionExports = &[export_c_func!(GSStatusBarHeight(_, _))];

pub const DYLIB: HostDylib = HostDylib {
    path: "/System/Library/PrivateFrameworks/GraphicsServices.framework/GraphicsServices",
    aliases: &[],
    class_exports: &[],
    constant_exports: &[],
    function_exports: &[FUNCTIONS],
};
