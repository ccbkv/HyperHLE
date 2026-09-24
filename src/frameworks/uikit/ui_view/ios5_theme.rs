/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Shared drawing primitives for the iOS 5/6 UIKit appearance.
//! Default appearance follows sketch-ios-master/Phone.svg and Clock.svg.
//!
//! iOS 5 controls are heavily *skeuomorphic*: bars and buttons are drawn with
//! a vertical two-part gloss gradient, a bright 1px highlight along the top
//! edge and a dark 1px shadow line along the bottom edge. This module provides
//! reusable primitives so the individual UIKit control implementations can
//! render that look consistently.
//!
//! The emulator's Core Graphics rasteriser only reliably rasterises axis
//! aligned rectangle fills and rectangle strokes (paths and real ellipses are
//! approximated). To stay within those guarantees, every gradient here is
//! synthesised as a stack of 1pt-tall rectangle "strips" whose colour is
//! linearly interpolated between the stops. This keeps the code fully
//! compatible with the existing renderer while still producing smooth
//! gradients on both Retina and non-Retina backing stores (the backing store
//! resolution is handled transparently by the context).

use crate::frameworks::core_graphics::cg_context::{
    CGContextClearRect, CGContextFillRect, CGContextRef, CGContextRestoreGState,
    CGContextSaveGState, CGContextSetRGBFillColor,
};
use crate::frameworks::core_graphics::{CGFloat, CGPoint, CGRect, CGSize};
use crate::Environment;
use crate::objc::{id, msg, msg_class, nil};
use crate::frameworks::core_graphics::cg_bitmap_context::{
    CGBitmapContextCreate, CGBitmapContextCreateImage,
};
use crate::frameworks::core_graphics::cg_context::CGContextRelease;
use crate::frameworks::core_graphics::cg_image::CGImageRelease;

/// A straight RGBA colour, components in the `0.0..=1.0` range.
pub type Rgba = (CGFloat, CGFloat, CGFloat, CGFloat);

#[inline]
fn lerp(a: CGFloat, b: CGFloat, t: CGFloat) -> CGFloat {
    a + (b - a) * t
}

#[inline]
fn lerp_rgba(a: Rgba, b: Rgba, t: CGFloat) -> Rgba {
    (
        lerp(a.0, b.0, t),
        lerp(a.1, b.1, t),
        lerp(a.2, b.2, t),
        lerp(a.3, b.3, t),
    )
}

/// PNG extracted from Phone.svg image0 (14 x 20 pixels, 7 x 10 points).
/// The reference SVG is not needed at build time or runtime.
pub fn draw_grouped_texture(env: &mut Environment, ctx: CGContextRef, rect: CGRect) {
    use std::sync::OnceLock;
    use crate::frameworks::core_graphics::cg_context::{CGContextClipToRect, CGContextDrawImage};
    static PIXELS: OnceLock<Vec<u8>> = OnceLock::new();
    if ctx.is_null() || !rect.size.width.is_finite() || !rect.size.height.is_finite()
        || rect.size.width <= 0.0 || rect.size.height <= 0.0 { return; }
    let pixels = PIXELS.get_or_init(|| {
        const PNG: &str = concat!(
            "iVBORw0KGgoAAAANSUhEUgAAAA4AAAAUCAYAAAC9BQwsAAAACXBIWXMAABYlAAAWJQFJUiTw",
            "AAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAAA1SURBVHgB7dSxDQBACAJA//P7r/",
            "aFrasYXMAYY2mghWs5Xw1SxD2tcWUYQkLCBfA1Ntl3IAAE8gmdW2ZFPQAAAABJRU5ErkJggg=="
        );
        let mut bytes = Vec::new();
        let mut bits = 0u32;
        let mut value = 0u32;
        for byte in PNG.bytes().take_while(|&b| b != b'=') {
            let n = match byte {
                b'A'..=b'Z' => byte - b'A', b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52, b'+' => 62, b'/' => 63,
                _ => unreachable!(),
            };
            value = (value << 6) | n as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                bytes.push((value >> bits) as u8);
                value &= (1 << bits) - 1;
            }
        }
        let image = crate::image::Image::from_bytes(&bytes).expect("embedded UIKit texture");
        assert_eq!(image.dimensions(), (14, 20));
        image.pixels().to_vec()
    });
    let image = crate::frameworks::core_graphics::cg_image::from_image(
        env, crate::image::Image::from_pixel_vec(pixels.clone(), (14, 20)),
    );
    CGContextSaveGState(env, ctx);
    CGContextClipToRect(env, ctx, rect);
    let x0 = (rect.origin.x / 7.0).floor() * 7.0;
    let y0 = (rect.origin.y / 10.0).floor() * 10.0;
    let columns = ((rect.origin.x + rect.size.width - x0) / 7.0).ceil() as i32;
    let rows = ((rect.origin.y + rect.size.height - y0) / 10.0).ceil() as i32;
    for y in 0..rows {
        for x in 0..columns {
            CGContextDrawImage(env, ctx, CGRect {
                origin: CGPoint { x: x0 + x as f32 * 7.0, y: y0 + y as f32 * 10.0 },
                size: CGSize { width: 7.0, height: 10.0 },
            }, image);
        }
    }
    CGContextRestoreGState(env, ctx);
    CGImageRelease(env, image);
}

/// Convert the SVG's sRGB colour notation without losing the source values.
pub const fn rgb(hex: u32) -> Rgba {
    (((hex >> 16) & 255) as CGFloat / 255.0,
     ((hex >> 8) & 255) as CGFloat / 255.0,
     (hex & 255) as CGFloat / 255.0, 1.0)
}

pub const NAVIGATION: [(CGFloat, Rgba); 4] = [
    (0.0, rgb(0xC1D1E4)), (0.33, rgb(0xA1B2C9)),
    (0.67, rgb(0x758CAB)), (1.0, rgb(0x557094)),
];
pub const NAVIGATION_BUTTON: [(CGFloat, Rgba); 4] = [
    (0.0, rgb(0xA2B2C9)), (0.33, rgb(0x798EAC)),
    (0.67, rgb(0x506D94)), (1.0, rgb(0x405F8A)),
];

fn sample(stops: &[(CGFloat, Rgba)], t: CGFloat) -> Rgba {
    let Some(&(mut position, mut color)) = stops.first() else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    for &(next, next_color) in &stops[1..] {
        if t < next {
            return lerp_rgba(color, next_color,
                ((t - position) / (next - position)).clamp(0.0, 1.0));
        }
        position = next;
        color = next_color;
    }
    color
}

/// Draw rounded scanlines directly; never erase neighbouring artwork.
pub fn draw_surface(
    env: &mut Environment, ctx: CGContextRef, rect: CGRect,
    radius: CGFloat, stops: &[(CGFloat, Rgba)], border: Rgba,
) {
    if ctx.is_null() || !rect.size.width.is_finite()
        || !rect.size.height.is_finite()
        || rect.size.width <= 0.0 || rect.size.height <= 0.0 { return; }
    CGContextSaveGState(env, ctx);
    let r = radius.max(0.0).min(rect.size.width / 2.0).min(rect.size.height / 2.0);
    let steps = (rect.size.height * 2.0).ceil() as i32;
    for i in 0..steps {
        let y = i as CGFloat * 0.5;
        let height = (rect.size.height - y).min(0.5);
        let cy = y + height / 2.0;
        let dy = (r - cy.min(rect.size.height - cy)).max(0.0);
        let inset = r - (r * r - dy * dy).max(0.0).sqrt();
        let t = if steps > 1 { i as CGFloat / (steps - 1) as CGFloat } else { 0.0 };
        let mut strip = CGRect {
            origin: CGPoint { x: rect.origin.x + inset, y: rect.origin.y + y },
            size: CGSize { width: (rect.size.width - 2.0 * inset).max(0.0), height },
        };
        let color = sample(stops, t);
        if border.3 > 0.0 {
            fill_solid(env, ctx, strip, border);
            if y < 0.5 || y + height > rect.size.height - 0.5 { continue; }
            strip.origin.x += 0.5;
            strip.size.width = (strip.size.width - 1.0).max(0.0);
        }
        fill_solid(env, ctx, strip, color);
    }
    CGContextRestoreGState(env, ctx);
}

/// Clamp a component to the representable colour range.
#[inline]
fn clamp01(x: CGFloat) -> CGFloat {
    x.clamp(0.0, 1.0)
}

/// Multiply the lightness of a colour by `factor` (values > 1 brighten,
/// < 1 darken). Alpha is preserved. Used to derive gloss stops from a single
/// bar tint colour.
#[inline]
pub fn scale_brightness(c: Rgba, factor: CGFloat) -> Rgba {
    (
        clamp01(c.0 * factor),
        clamp01(c.1 * factor),
        clamp01(c.2 * factor),
        c.3,
    )
}

/// Fill `rect` with a solid colour.
pub fn fill_solid(env: &mut Environment, ctx: CGContextRef, rect: CGRect, color: Rgba) {
    if ctx.is_null() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    CGContextSetRGBFillColor(env, ctx, color.0, color.1, color.2, color.3);
    CGContextFillRect(env, ctx, rect);
}

/// Fill `rect` with a smooth top-to-bottom linear gradient between `top` and
/// `bottom`. Rendered as a stack of 1pt strips (see the module docs).
pub fn fill_vertical_gradient(
    env: &mut Environment,
    ctx: CGContextRef,
    rect: CGRect,
    top: Rgba,
    bottom: Rgba,
) {
    if ctx.is_null() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    let steps = rect.size.height.ceil().max(1.0) as i32;
    for i in 0..steps {
        let t = if steps > 1 {
            i as CGFloat / (steps - 1) as CGFloat
        } else {
            0.0
        };
        let (r, g, b, a) = lerp_rgba(top, bottom, t);
        CGContextSetRGBFillColor(env, ctx, r, g, b, a);
        let strip = CGRect {
            origin: CGPoint {
                x: rect.origin.x,
                y: rect.origin.y + i as CGFloat,
            },
            size: CGSize {
                width: rect.size.width,
                height: (rect.size.height - i as CGFloat).min(1.0),
            },
        };
        CGContextFillRect(env, ctx, strip);
    }
}

/// Draw a horizontal hairline (1pt tall by default) at `y` spanning the width
/// of `rect`. Used for the bright highlight / dark shadow edges of bars.
pub fn horizontal_line(
    env: &mut Environment,
    ctx: CGContextRef,
    rect: CGRect,
    y: CGFloat,
    color: Rgba,
    thickness: CGFloat,
) {
    fill_solid(
        env,
        ctx,
        CGRect {
            origin: CGPoint { x: rect.origin.x, y },
            size: CGSize {
                width: rect.size.width,
                height: thickness,
            },
        },
        color,
    );
}

/// The colour recipe for an iOS 5 bar (`UINavigationBar`, `UIToolbar`,
/// `UITabBar`). All values are hand-tuned to match iOS 5 screenshots.
#[derive(Clone, Copy)]
pub struct BarPalette {
    /// Bright highlight painted along the very top edge (1px).
    pub top_highlight: Rgba,
    /// Gradient stops for the upper (glossier) half of the bar.
    pub upper_top: Rgba,
    pub upper_bottom: Rgba,
    /// Gradient stops for the lower half of the bar.
    pub lower_top: Rgba,
    pub lower_bottom: Rgba,
    /// Dark shadow line painted along the very bottom edge (1px).
    pub bottom_shadow: Rgba,
    pub stop_positions: [CGFloat; 4],
}

impl BarPalette {
    /// The default blue-grey metallic tint used by `UINavigationBar` and
    /// `UIToolbar` on iOS 5.
    pub fn navigation_default() -> Self {
        BarPalette {
            top_highlight: rgb(0xD6E1EF),
            upper_top: NAVIGATION[0].1,
            upper_bottom: NAVIGATION[1].1,
            lower_top: NAVIGATION[2].1,
            lower_bottom: NAVIGATION[3].1,
            bottom_shadow: rgb(0x3F5C80),
            stop_positions: [0.0, 0.33, 0.67, 1.0],
        }
    }

    /// `UIBarStyleBlack` bars (also the base for the `UITabBar`).
    pub fn black() -> Self {
        BarPalette {
            top_highlight: (0.39, 0.39, 0.40, 1.0),
            upper_top: (0.29, 0.29, 0.30, 1.0),
            upper_bottom: (0.18, 0.18, 0.19, 1.0),
            lower_top: (0.18, 0.18, 0.19, 1.0),
            lower_bottom: (0.07, 0.07, 0.08, 1.0),
            bottom_shadow: (0.0, 0.0, 0.0, 1.0),
            stop_positions: [0.0, 0.33, 0.67, 1.0],
        }
    }

    /// The light-grey gradient used behind a `UISearchBar` on iOS 5.
    pub fn search_bar() -> Self {
        BarPalette {
            top_highlight: rgb(0xE6EBEF),
            upper_top: rgb(0xD6DDE2),
            upper_bottom: rgb(0xCCD4D9),
            lower_top: rgb(0xBDC7CD),
            lower_bottom: rgb(0xB2BDC4),
            bottom_shadow: rgb(0x7C8FA4),
            stop_positions: [0.0, 0.33, 0.67, 1.0],
        }
    }

    /// The dark, glossy `UITabBar` background.
    pub fn tab_bar() -> Self {
        BarPalette {
            top_highlight: rgb(0x555555),
            upper_top: (0.20, 0.20, 0.20, 1.0),
            upper_bottom: (0.08, 0.08, 0.08, 1.0),
            lower_top: rgb(0x000000),
            lower_bottom: rgb(0x000000),
            bottom_shadow: rgb(0x000000),
            stop_positions: [0.0, 0.510204, 0.510304, 1.0],
        }
    }

    /// Derive a glossy palette from a single flat bar tint colour, mirroring
    /// how UIKit lightens/darkens `barTintColor`/`tintColor` to build the
    /// gradient.
    pub fn from_tint(tint: Rgba) -> Self {
        BarPalette {
            top_highlight: lerp_rgba(tint, (1.0, 1.0, 1.0, tint.3), 0.55),
            upper_top: lerp_rgba(tint, (1.0, 1.0, 1.0, tint.3), 0.30),
            upper_bottom: scale_brightness(tint, 0.95),
            lower_top: scale_brightness(tint, 0.95),
            lower_bottom: scale_brightness(tint, 0.58),
            bottom_shadow: scale_brightness(tint, 0.30),
            stop_positions: [0.0, 0.33, 0.67, 1.0],
        }
    }

    /// Apply an alpha multiplier to every stop (used for translucent bars).
    pub fn with_alpha(mut self, alpha: CGFloat) -> Self {
        for c in [
            &mut self.top_highlight,
            &mut self.upper_top,
            &mut self.upper_bottom,
            &mut self.lower_top,
            &mut self.lower_bottom,
            &mut self.bottom_shadow,
        ] {
            c.3 *= alpha;
        }
        self
    }
}

/// Render the full iOS 5 bar chrome (two-part gloss gradient plus top
/// highlight and bottom shadow lines) into `rect`.
pub fn draw_bar_background(
    env: &mut Environment,
    ctx: CGContextRef,
    rect: CGRect,
    palette: BarPalette,
) {
    if ctx.is_null() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    CGContextSaveGState(env, ctx);
    let p = palette.stop_positions;
    draw_surface(env, ctx, rect, 0.0, &[
        (p[0], palette.upper_top), (p[1], palette.upper_bottom),
        (p[2], palette.lower_top), (p[3], palette.lower_bottom),
    ], (0.0, 0.0, 0.0, 0.0));
    // Bright highlight along the top edge.
    horizontal_line(env, ctx, rect, rect.origin.y, palette.top_highlight, 1.0);
    // Dark shadow along the bottom edge.
    horizontal_line(
        env,
        ctx,
        rect,
        rect.origin.y + rect.size.height - 1.0,
        palette.bottom_shadow,
        1.0,
    );
    CGContextRestoreGState(env, ctx);
}

/// Render an iOS 5 style glossy pill/button background inside `rect`. The
/// renderer cannot rasterise rounded corners into a bitmap, so callers that
/// need rounded corners should additionally set their layer's `cornerRadius`
/// (which the compositor rounds); this fills the glossy body.
pub fn draw_glossy_button(
    env: &mut Environment,
    ctx: CGContextRef,
    rect: CGRect,
    base: Rgba,
    highlighted: bool,
) {
    if ctx.is_null() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    CGContextSaveGState(env, ctx);
    let factor = if highlighted { 0.72 } else { 1.0 };
    let white = (1.0, 1.0, 1.0, base.3);
    let top = scale_brightness(lerp_rgba(base, white, 0.4), factor);
    let upper_bottom = scale_brightness(lerp_rgba(base, white, 0.1), factor);
    let lower_top = scale_brightness(base, factor);
    let bottom = scale_brightness(base, 0.80 * factor);
    let mid = (rect.size.height / 2.0).floor();
    let upper = CGRect {
        origin: rect.origin,
        size: CGSize {
            width: rect.size.width,
            height: mid,
        },
    };
    let lower = CGRect {
        origin: CGPoint {
            x: rect.origin.x,
            y: rect.origin.y + mid,
        },
        size: CGSize {
            width: rect.size.width,
            height: rect.size.height - mid,
        },
    };
    fill_vertical_gradient(env, ctx, upper, top, upper_bottom);
    fill_vertical_gradient(env, ctx, lower, lower_top, bottom);
    // Inner top highlight for the glass "shine".
    horizontal_line(
        env,
        ctx,
        rect,
        rect.origin.y,
        scale_brightness(base, 1.5 * factor),
        1.0,
    );
    CGContextRestoreGState(env, ctx);
}

/// Draw an iOS 6 navigation button without clearing the bar underneath.
pub fn draw_navigation_button(
    env: &mut Environment,
    ctx: CGContextRef,
    rect: CGRect,
    base: Rgba,
    back: bool,
    highlighted: bool,
) {
    if ctx.is_null() || rect.size.width < 4.0 || rect.size.height < 4.0 {
        return;
    }
    CGContextSaveGState(env, ctx);
    let h = rect.size.height;
    let radius = 5.0f32.min(h / 2.0);
    let factor = if highlighted { 0.68 } else { 1.0 };
    // Preserve caller tint while matching the SVG's navigation-button stops.
    let reference = (0.36, 0.46, 0.62, 1.0);
    let stops = NAVIGATION_BUTTON.map(|(p, c)| (p, (
        (c.0 + base.0 - reference.0).clamp(0.0, 1.0),
        (c.1 + base.1 - reference.1).clamp(0.0, 1.0),
        (c.2 + base.2 - reference.2).clamp(0.0, 1.0), base.3,
    )));
    for i in 0..h.ceil() as i32 {
        let y = i as CGFloat;
        let edge = (y + 0.5).min(h - y - 0.5).max(0.0);
        let dy = (radius - edge).max(0.0);
        let round = radius - (radius * radius - dy * dy).max(0.0).sqrt();
        let left = if back {
            ((y + 0.5 - h / 2.0).abs() / (h / 2.0)) * 11.0
        } else { round };
        let strip = CGRect {
            origin: CGPoint { x: rect.origin.x + left, y: rect.origin.y + y },
            size: CGSize {
                width: (rect.size.width - left - round).max(0.0),
                height: (h - y).min(1.0),
            },
        };
        fill_solid(env, ctx, strip, scale_brightness(base, 0.36));
        if i > 0 && y + 1.0 < h && strip.size.width > 2.0 {
            let color = if i == 1 {
                lerp_rgba(base, (1.0, 1.0, 1.0, base.3), 0.5)
            } else {
                sample(&stops, y / (h - 1.0))
            };
            let body = CGRect {
                origin: CGPoint { x: strip.origin.x + 1.0, y: strip.origin.y },
                size: CGSize { width: strip.size.width - 2.0, height: strip.size.height },
            };
            fill_solid(env, ctx, body, scale_brightness(color, factor));
        }
    }
    CGContextRestoreGState(env, ctx);
}

/// Paint an internal control view's layer at its current size.
pub fn set_surface(
    env: &mut Environment, view: id, radius: CGFloat,
    stops: &[(CGFloat, Rgba)], border: Rgba,
) {
    let bounds: CGRect = msg![env; view bounds];
    let size = bounds.size;
    if !size.width.is_finite() || !size.height.is_finite()
        || size.width <= 0.0 || size.height <= 0.0
        || size.width > 4096.0 || size.height > 4096.0 { return; }
    let width = size.width.ceil() as u32;
    let height = size.height.ceil() as u32;
    let ctx = CGBitmapContextCreate(env, crate::mem::MutPtr::null(),
        width, height, 8, width * 4, nil, 0x0002_0001);
    if ctx.is_null() { return; }
    let rect = CGRect { origin: CGPoint::default(), size };
    CGContextClearRect(env, ctx, rect);
    draw_surface(env, ctx, rect, radius, stops, border);
    let image = CGBitmapContextCreateImage(env, ctx);
    let layer: id = msg![env; view layer];
    let clear: id = msg_class![env; UIColor clearColor];
    () = msg![env; view setBackgroundColor:clear];
    () = msg![env; layer setContents:image];
    CGImageRelease(env, image);
    CGContextRelease(env, ctx);
}

/// Inset a rectangle by `dx`/`dy` on every edge.
#[inline]
pub fn inset_rect(rect: CGRect, dx: CGFloat, dy: CGFloat) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: rect.origin.x + dx,
            y: rect.origin.y + dy,
        },
        size: CGSize {
            width: (rect.size.width - dx * 2.0).max(0.0),
            height: (rect.size.height - dy * 2.0).max(0.0),
        },
    }
}

/// Clear (make transparent) the four corner regions of `rect` so that content
/// already drawn into `rect` appears with rounded corners of the given
/// `radius`. The renderer cannot rasterise real arcs, so the arc is
/// approximated per scanline with horizontal clears — this looks smooth at
/// typical control sizes and works identically at Retina and non-Retina
/// backing-store resolutions.
///
/// This is how every rounded iOS 5 control in this theme obtains its rounded
/// silhouette without requiring the CoreAnimation compositor to clip drawn
/// layer `contents` to `cornerRadius`.
pub fn clear_rounded_corners(env: &mut Environment, ctx: CGContextRef, rect: CGRect, radius: CGFloat) {
    if ctx.is_null() {
        return;
    }
    let r = radius
        .min(rect.size.width / 2.0)
        .min(rect.size.height / 2.0);
    if r <= 0.5 {
        return;
    }
    let steps = r.ceil() as i32;
    let right_x = rect.origin.x + rect.size.width;
    let bottom_y = rect.origin.y + rect.size.height;
    for i in 0..steps {
        let y = i as CGFloat;
        // Horizontal distance from the arc centre for this scanline.
        let dy = r - y - 0.5;
        let inset = r - (r * r - dy * dy).max(0.0).sqrt();
        if inset <= 0.0 {
            continue;
        }
        // Top-left + top-right.
        CGContextClearRect(
            env,
            ctx,
            CGRect {
                origin: CGPoint {
                    x: rect.origin.x,
                    y: rect.origin.y + y,
                },
                size: CGSize {
                    width: inset,
                    height: 1.0,
                },
            },
        );
        CGContextClearRect(
            env,
            ctx,
            CGRect {
                origin: CGPoint {
                    x: right_x - inset,
                    y: rect.origin.y + y,
                },
                size: CGSize {
                    width: inset,
                    height: 1.0,
                },
            },
        );
        // Bottom-left + bottom-right.
        CGContextClearRect(
            env,
            ctx,
            CGRect {
                origin: CGPoint {
                    x: rect.origin.x,
                    y: bottom_y - y - 1.0,
                },
                size: CGSize {
                    width: inset,
                    height: 1.0,
                },
            },
        );
        CGContextClearRect(
            env,
            ctx,
            CGRect {
                origin: CGPoint {
                    x: right_x - inset,
                    y: bottom_y - y - 1.0,
                },
                size: CGSize {
                    width: inset,
                    height: 1.0,
                },
            },
        );
    }
}

/// Draw an iOS 5 glossy, rounded button/pill fully inside `rect`: a 1px darker
/// bezel, the glossy body and rounded corners. `base` is the button's fill
/// colour; pass `highlighted = true` for the pressed (darkened) state.
pub fn draw_rounded_glossy_button(
    env: &mut Environment,
    ctx: CGContextRef,
    rect: CGRect,
    base: Rgba,
    radius: CGFloat,
    highlighted: bool,
) {
    if ctx.is_null() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    CGContextSaveGState(env, ctx);
    draw_surface(env, ctx, rect, radius,
        &[(0.0, rgb(0x333435)), (1.0, rgb(0x737374))],
        (0.0, 0.0, 0.0, 0.0));
    let body = inset_rect(rect, 1.0, 1.0);
    let factor = if highlighted { 0.72 } else { 1.0 };
    let white = (1.0, 1.0, 1.0, base.3);
    draw_surface(env, ctx, body, (radius - 1.0).max(0.0), &[
        (0.0, scale_brightness(lerp_rgba(base, white, 0.4), factor)),
        (0.5, scale_brightness(lerp_rgba(base, white, 0.1), factor)),
        (0.5, scale_brightness(base, factor)),
        (1.0, scale_brightness(base, factor * 0.8)),
    ], (1.0, 1.0, 1.0, 0.2));
    CGContextRestoreGState(env, ctx);
}

/// Draw a recessed (inset) rounded track, as used by `UISlider`,
/// `UIProgressView` grooves and search fields. `fill` is the groove colour.
pub fn draw_recessed_track(
    env: &mut Environment,
    ctx: CGContextRef,
    rect: CGRect,
    fill: Rgba,
    radius: CGFloat,
) {
    if ctx.is_null() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    CGContextSaveGState(env, ctx);
    draw_surface(env, ctx, rect, radius, &[
        (0.0, scale_brightness(fill, 0.65)),
        (0.18, scale_brightness(fill, 0.86)),
        (0.8, fill),
        (1.0, lerp_rgba(fill, (1.0, 1.0, 1.0, fill.3), 0.35)),
    ], scale_brightness(fill, 0.55));
    CGContextRestoreGState(env, ctx);
}

/// Present a themed modal panel. The caller owns the returned overlay.
pub fn present_panel(
    env: &mut Environment, owner: id, action: crate::objc::SEL,
    title: id, message: id, titles: id, destructive: i32, sheet: bool,
) -> id {
    let app: id = msg_class![env; UIApplication sharedApplication];
    let window: id = msg![env; app keyWindow];
    if window == nil { return nil; }
    let bounds: CGRect = msg![env; window bounds];
    let overlay: id = msg_class![env; UIView alloc];
    let overlay: id = msg![env; overlay initWithFrame:bounds];
    let dim: id = msg_class![env; UIColor colorWithWhite:0.0f32 alpha:0.45f32];
    () = msg![env; overlay setBackgroundColor:dim];
    let count: u32 = msg![env; titles count];
    let width = (bounds.size.width - 24.0).max(1.0).min(if sheet { 480.0 } else { 284.0 });
    let label_width = (width - 24.0).max(1.0);
    let mut labels = Vec::new();
    let mut y = 14.0;
    for (text, size) in [(title, 18.0f32), (message, 14.0f32)] {
        let length: u32 = msg![env; text length];
        if length == 0 { continue; }
        let label: id = msg_class![env; UILabel new];
        let font: id = msg_class![env; UIFont boldSystemFontOfSize:size];
        let white: id = msg_class![env; UIColor whiteColor];
        let clear: id = msg_class![env; UIColor clearColor];
        let shadow: id = msg_class![env; UIColor blackColor];
        () = msg![env; label setText:text];
        () = msg![env; label setFont:font];
        () = msg![env; label setTextColor:white];
        () = msg![env; label setBackgroundColor:clear];
        () = msg![env; label setTextAlignment:1i32];
        () = msg![env; label setNumberOfLines:0i32];
        () = msg![env; label setShadowColor:shadow];
        () = msg![env; label setShadowOffset:(CGSize { width: 0.0, height: -1.0 })];
        let fit: CGSize = msg![env; label sizeThatFits:(CGSize { width: label_width, height: 10000.0 })];
        let h = fit.height.max(size + 4.0);
        () = msg![env; label setFrame:(CGRect { origin: CGPoint { x: 12.0, y }, size: CGSize { width: label_width, height: h } })];
        y += h + 8.0;
        labels.push(label);
    }
    let height = y + count as f32 * 48.0 + 8.0;
    let visible_height = height.min((bounds.size.height - 24.0).max(1.0));
    let panel: id = msg_class![env; UIScrollView alloc];
    let frame = CGRect {
        origin: CGPoint { x: (bounds.size.width - width) / 2.0,
            y: if sheet { bounds.size.height - visible_height } else { (bounds.size.height - visible_height) / 2.0 } },
        size: CGSize { width, height: visible_height },
    };
    let panel: id = msg![env; panel initWithFrame:frame];
    () = msg![env; panel setContentSize:(CGSize { width, height })];
    set_surface(env, panel, 10.0,
        &[(0.0, rgb(0x4C576B)), (0.5, rgb(0x252F42)), (1.0, rgb(0x111621))], rgb(0xB8BEC5));
    for label in labels { () = msg![env; panel addSubview:label]; crate::objc::release(env, label); }
    for i in 0..count {
        let title: id = msg![env; titles objectAtIndex:i];
        let button: id = msg_class![env; UIButton buttonWithType:0i32];
        () = msg![env; button setFrame:(CGRect { origin: CGPoint { x: 8.0, y: y + i as f32 * 48.0 }, size: CGSize { width: width - 16.0, height: 42.0 } })];
        () = msg![env; button setTitle:title forState:0u32];
        () = msg![env; button setTag:(i as i32)];
        () = msg![env; button addTarget:owner action:action forControlEvents:64u32];
        let base = if i as i32 == destructive { rgb(0xB52C2C) } else { rgb(0x506D94) };
        set_surface(env, button, 8.0,
            &[(0.0, scale_brightness(base, 1.4)), (0.5, base), (1.0, scale_brightness(base, 0.7))], rgb(0x333435));
        let white: id = msg_class![env; UIColor whiteColor];
        () = msg![env; button setTitleColor:white forState:0u32];
        () = msg![env; panel addSubview:button];
    }
    () = msg![env; overlay addSubview:panel];
    crate::objc::release(env, panel);
    () = msg![env; window addSubview:overlay];
    overlay
}

/// Palette + renderer for the iOS 5 dark, glossy rounded panel shared by
/// `UIAlertView` and `UIActionSheet`.
pub fn draw_alert_panel(env: &mut Environment, ctx: CGContextRef, rect: CGRect, radius: CGFloat) {
    if ctx.is_null() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    CGContextSaveGState(env, ctx);
    // Light outer stroke ("rim light") around the panel.
    fill_solid(env, ctx, rect, (0.72, 0.74, 0.78, 0.95));
    let body = inset_rect(rect, 1.0, 1.0);
    // Dark blue-charcoal glossy body: brighter glassy upper third, darker
    // lower body, matching the classic iOS 5 alert.
    let upper_h = (body.size.height * 0.5).floor();
    let upper = CGRect {
        origin: body.origin,
        size: CGSize {
            width: body.size.width,
            height: upper_h,
        },
    };
    let lower = CGRect {
        origin: CGPoint {
            x: body.origin.x,
            y: body.origin.y + upper_h,
        },
        size: CGSize {
            width: body.size.width,
            height: body.size.height - upper_h,
        },
    };
    fill_vertical_gradient(
        env,
        ctx,
        upper,
        (0.30, 0.34, 0.42, 0.98),
        (0.16, 0.19, 0.26, 0.98),
    );
    fill_vertical_gradient(
        env,
        ctx,
        lower,
        (0.13, 0.15, 0.21, 0.98),
        (0.07, 0.08, 0.12, 0.98),
    );
    // Glass shine on the very top.
    horizontal_line(env, ctx, body, body.origin.y, (0.5, 0.54, 0.62, 0.9), 1.0);
    clear_rounded_corners(env, ctx, rect, radius);
    CGContextRestoreGState(env, ctx);
}