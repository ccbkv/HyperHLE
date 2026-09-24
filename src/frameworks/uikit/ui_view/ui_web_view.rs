/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0.
 * If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UIWebView`.

use crate::frameworks::core_graphics::{cg_image, CGRect};
use crate::frameworks::foundation::ns_string::{self, to_rust_string};
use crate::frameworks::foundation::NSUInteger;
use crate::frameworks::uikit::ui_view::UIViewHostObject;
use crate::image::Image;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release, retain,
    ClassExports, NSZonePtr,
};
use crate::Environment;
use std::path::PathBuf;
use std::process::Command;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use crate::fs::GuestPath;

#[derive(Clone)]
struct WebPage {
    url: String,
    body: Option<(Vec<u8>, String)>,
}
use std::sync::atomic::{AtomicUsize, Ordering};

// UIWebViewNavigationType constants
pub type UIWebViewNavigationType = i32;
pub const UIWebViewNavigationTypeLinkClicked: UIWebViewNavigationType = 0;
pub const UIWebViewNavigationTypeFormSubmitted: UIWebViewNavigationType = 1;
pub const UIWebViewNavigationTypeBackForward: UIWebViewNavigationType = 2;
pub const UIWebViewNavigationTypeReload: UIWebViewNavigationType = 3;
pub const UIWebViewNavigationTypeFormResubmitted: UIWebViewNavigationType = 4;
pub const UIWebViewNavigationTypeOther: UIWebViewNavigationType = 5;

// UIDataDetectorTypes bitmask
pub type UIDataDetectorTypes = NSUInteger;
pub const UIDataDetectorTypePhoneNumber: UIDataDetectorTypes = 1 << 0;
pub const UIDataDetectorTypeLink: UIDataDetectorTypes = 1 << 1;
pub const UIDataDetectorTypeAddress: UIDataDetectorTypes = 1 << 2;
pub const UIDataDetectorTypeCalendarEvent: UIDataDetectorTypes = 1 << 3;
pub const UIDataDetectorTypeNone: UIDataDetectorTypes = 0;
pub const UIDataDetectorTypeAll: UIDataDetectorTypes = u32::MAX as UIDataDetectorTypes;

#[derive(Default)]
struct UIWebViewHostObject {
    superclass: UIViewHostObject,
    /// UIWebViewDelegate — weak reference (no retain per Apple docs)
    delegate: id,
    scales_page_to_fit: bool,
    detects_phone_numbers: bool,
    data_detector_types: UIDataDetectorTypes,
    allows_inline_media_playback: bool,
    media_playback_requires_user_action: bool,
    media_playback_allows_air_play: bool,
    suppress_incremental_rendering: bool,
    keyboard_display_requires_user_action: bool,
    pagination_mode: i32,
    pagination_breaking_mode: i32,
    page_length: f64,
    gap_between_pages: f64,
    /// NSString* — last URL string passed to loadRequest:
    current_url: id,

    loading: bool,
    load_revision: u64,
    current_page: Option<WebPage>,
    back_stack: Vec<WebPage>,
    forward_stack: Vec<WebPage>,
}
impl_HostObject_with_superclass!(UIWebViewHostObject);

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UIWebView: UIView

// =========================================================================
// MARK: - Allocation
// =========================================================================

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::new(UIWebViewHostObject {
        superclass: UIViewHostObject::default(),
        delegate: nil,
        scales_page_to_fit: false,
        detects_phone_numbers: true,
        data_detector_types: UIDataDetectorTypePhoneNumber,
        allows_inline_media_playback: false,
        media_playback_requires_user_action: true,
        media_playback_allows_air_play: true,
        suppress_incremental_rendering: false,
        keyboard_display_requires_user_action: true,

        pagination_mode: 0,
        pagination_breaking_mode: 0,
        page_length: 0.0,
        gap_between_pages: 0.0,
        current_url: nil,
        loading: false,
        load_revision: 0,
        current_page: None,
        back_stack: Vec::new(),
        forward_stack: Vec::new(),
    });
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

// =========================================================================
// MARK: - Initializers
// =========================================================================

- (id)init {
    msg_super![env; this init]
}

- (id)initWithFrame:(CGRect)frame {
    msg_super![env; this initWithFrame:frame]
}

- (id)initWithCoder:(id)coder {
    msg_super![env; this initWithCoder:coder]
}

// =========================================================================
// MARK: - Dealloc
// =========================================================================

- (())dealloc {
    let current_url = env.objc.borrow::<UIWebViewHostObject>(this).current_url;
    release(env, current_url);
    msg_super![env; this dealloc]
}

// =========================================================================
// MARK: - Delegate
// =========================================================================

- (id)delegate {
    env.objc.borrow::<UIWebViewHostObject>(this).delegate
}

- (())setDelegate:(id)delegate {
    // Weak reference — do NOT retain.
    env.objc.borrow_mut::<UIWebViewHostObject>(this).delegate = delegate;
}

// =========================================================================
// MARK: - Loading
// =========================================================================

- (())loadRequest:(id)request { // NSURLRequest*
    let url_string: String = if request != nil {
        let url: id = msg![env; request URL];
        let url_desc: id = msg![env; url description];
        if url_desc != nil { to_rust_string(env, url_desc).into_owned() } else { String::new() }
    } else {
        String::new()
    };
    log!("UIWebView loadRequest: {}", url_string);

    load_page(env, this, WebPage { url: url_string, body: None }, 0);
}

- (())loadHTMLString:(id)html baseURL:(id)base_url {
    let bytes = if html == nil { Vec::new() } else {
        to_rust_string(env, html).as_bytes().to_vec()
    };
    let page = page_from_data(env, bytes, "text/html; charset=utf-8".into(), base_url);
    load_page(env, this, page, 0);
}

- (())loadData:(id)data MIMEType:(id)mime
      textEncodingName:(id)enc baseURL:(id)base_url {
    let bytes = if data == nil { Vec::new() } else {
        crate::frameworks::foundation::ns_data::to_rust_slice(env, data).to_vec()
    };
    let mut content_type = if mime == nil { "text/html".into() } else {
        to_rust_string(env, mime).into_owned()
    };
    if enc != nil {
        content_type.push_str("; charset=");
        content_type.push_str(&to_rust_string(env, enc));
    }
    let page = page_from_data(env, bytes, content_type, base_url);
    load_page(env, this, page, 0);
}

// =========================================================================
// MARK: - Subview management
// =========================================================================

- (())insertSubview:(id)view aboveSubview:(id)sibling {
    if view == nil { return; }

    // If sibling is nil or not in our view hierarchy, just add at the top.
    if sibling == nil {
        let _: () = msg![env; this addSubview:view];
        return;
    }

    // Delegate to UIView's insertSubview:aboveSubview: on our own view.
    let self_view: id = msg![env; this view];
    if self_view != nil {
        let _: () = msg![env; self_view insertSubview:view aboveSubview:sibling];
    } else {
        // Fallback — just add it.
        let _: () = msg![env; this addSubview:view];
    }
}

- (())reload {
    let page = env.objc.borrow::<UIWebViewHostObject>(this).current_page.clone();
    if let Some(page) = page { load_page(env, this, page, 3); }
}

- (())stopLoading {
    let host = env.objc.borrow_mut::<UIWebViewHostObject>(this);
    if !host.loading { return; }
    host.loading = false;
    host.load_revision = host.load_revision.wrapping_add(1);
    notify_load(env, this, Some((-999, "Loading cancelled".into())));
}

- (bool)isLoading {
    env.objc.borrow::<UIWebViewHostObject>(this).loading
}

// =========================================================================
// MARK: - Navigation
// =========================================================================

- (bool)canGoBack {
    !env.objc.borrow::<UIWebViewHostObject>(this).back_stack.is_empty()
}

- (bool)canGoForward {
    !env.objc.borrow::<UIWebViewHostObject>(this).forward_stack.is_empty()
}

- (())goBack {
    let page = env.objc.borrow::<UIWebViewHostObject>(this).back_stack.last().cloned();
    if let Some(page) = page { load_page(env, this, page, 1); }
}

- (())goForward {
    let page = env.objc.borrow::<UIWebViewHostObject>(this).forward_stack.last().cloned();
    if let Some(page) = page { load_page(env, this, page, 2); }
}

// =========================================================================
// MARK: - JavaScript
// =========================================================================

- (id)stringByEvaluatingJavaScriptFromString:(id)script { // NSString* -> NSString*
    let script_str = if script != nil {
        to_rust_string(env, script).into_owned()
    } else { String::new() };
    log_dbg!("UIWebView stringByEvaluatingJavaScriptFromString: {:?} — returning empty string", script_str);
    // Return empty NSString rather than nil — some apps check the return value.
    let empty = ns_string::from_rust_string(env, String::new());
    crate::objc::autorelease(env, empty)
}

// =========================================================================
// MARK: - Request / URL accessors
// =========================================================================

- (id)request { // NSURLRequest*
    let current = env.objc.borrow::<UIWebViewHostObject>(this).current_url;
    if current == nil { return nil; }
    let text = to_rust_string(env, current).into_owned();
    request_for_url(env, &text)
}

// Returns the URL of the currently loaded page as an NSString*.
- (id)_currentURLString { // NSString* (private helper)
    env.objc.borrow::<UIWebViewHostObject>(this).current_url
}

// =========================================================================
// MARK: - Properties
// =========================================================================

- (bool)scalesPageToFit {
    env.objc.borrow::<UIWebViewHostObject>(this).scales_page_to_fit
}
- (())setScalesPageToFit:(bool)scales {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).scales_page_to_fit = scales;
}

- (bool)detectsPhoneNumbers {
    env.objc.borrow::<UIWebViewHostObject>(this).detects_phone_numbers
}
- (())setDetectsPhoneNumbers:(bool)value {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).detects_phone_numbers = value;
}

- (UIDataDetectorTypes)dataDetectorTypes {
    env.objc.borrow::<UIWebViewHostObject>(this).data_detector_types
}
- (())setDataDetectorTypes:(UIDataDetectorTypes)types {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).data_detector_types = types;
}

- (bool)allowsInlineMediaPlayback {
    env.objc.borrow::<UIWebViewHostObject>(this).allows_inline_media_playback
}
- (())setAllowsInlineMediaPlayback:(bool)value {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).allows_inline_media_playback = value;
}

- (bool)mediaPlaybackRequiresUserAction {
    env.objc.borrow::<UIWebViewHostObject>(this).media_playback_requires_user_action
}
- (())setMediaPlaybackRequiresUserAction:(bool)value {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).media_playback_requires_user_action = value;
}

- (bool)mediaPlaybackAllowsAirPlay {
    env.objc.borrow::<UIWebViewHostObject>(this).media_playback_allows_air_play
}
- (())setMediaPlaybackAllowsAirPlay:(bool)value {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).media_playback_allows_air_play = value;
}

- (bool)suppressesIncrementalRendering {
    env.objc.borrow::<UIWebViewHostObject>(this).suppress_incremental_rendering
}
- (())setSuppressesIncrementalRendering:(bool)value {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).suppress_incremental_rendering = value;
}

- (bool)keyboardDisplayRequiresUserAction {
    env.objc.borrow::<UIWebViewHostObject>(this).keyboard_display_requires_user_action
}
- (())setKeyboardDisplayRequiresUserAction:(bool)value {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).keyboard_display_requires_user_action = value;
}

// Pagination (iOS 7+)
- (i32)paginationMode { // UIWebPaginationMode
    env.objc.borrow::<UIWebViewHostObject>(this).pagination_mode
}
- (())setPaginationMode:(i32)mode {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).pagination_mode = mode;
}

- (i32)paginationBreakingMode { // UIWebPaginationBreakingMode
    env.objc.borrow::<UIWebViewHostObject>(this).pagination_breaking_mode
}
- (())setPaginationBreakingMode:(i32)mode {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).pagination_breaking_mode = mode;
}

- (f64)pageLength {
    env.objc.borrow::<UIWebViewHostObject>(this).page_length
}
- (())setPageLength:(f64)length {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).page_length = length;
}

- (f64)gapBetweenPages {
    env.objc.borrow::<UIWebViewHostObject>(this).gap_between_pages
}
- (())setGapBetweenPages:(f64)gap {
    env.objc.borrow_mut::<UIWebViewHostObject>(this).gap_between_pages = gap;
}

- (u32)pageCount { // NSUInteger
    // No real rendering — always 0.
    0u32
}

// =========================================================================
// MARK: - Scroll view
// =========================================================================

// Returns a stub scroll view so apps that access scrollView don't crash.
- (id)scrollView { // UIScrollView*
    // Return self as a passthrough — we don't have a real UIScrollView here.
    this
}

// =========================================================================
// MARK: - Description
// =========================================================================

- (id)description {
    let (loading, current_url) = {
        let h = env.objc.borrow::<UIWebViewHostObject>(this);
        (h.loading, h.current_url)
    };
    let url_str = if current_url != nil {
        to_rust_string(env, current_url).into_owned()
    } else { "(nil)".into() };
    let s = format!(
        "<UIWebView: {:?}; loading={}; url={}>",
        this, loading, url_str
    );
    let cstr = env.mem.alloc_and_write_cstr(s.as_bytes());
    msg_class![env; NSString stringWithUTF8String:cstr]
}

@end

};

// =========================================================================
// MARK: - Chromium/CDP bridge: render a URL into the view's layer.contents
// =========================================================================
//
// touchHLE has no HTML rendering engine. As an opportunistic fallback (see
// PR description) we shell out to the host's headless Chromium to rasterise
// the target URL into a PNG, then install that PNG as the CALayer contents
// for the UIWebView. This gives apps like Google Mobile a visible web page
// instead of a blank rectangle, at the cost of interactivity.

fn request_for_url(env: &mut Environment, text: &str) -> id {
    let string = ns_string::from_rust_string(env, text.into());
    let url: id = if text.starts_with('/') {
        msg_class![env; NSURL fileURLWithPath:string]
    } else { msg_class![env; NSURL URLWithString:string] };
    release(env, string);
    msg_class![env; NSURLRequest requestWithURL:url]
}

fn notify_load(env: &mut Environment, view: id, error: Option<(i32, String)>) {
    let delegate = env.objc.borrow::<UIWebViewHostObject>(view).delegate;
    retain(env, delegate);
    let name = if error.is_some() { "webView:didFailLoadWithError:" }
        else { "webViewDidFinishLoad:" };
    let sel = env.objc.register_host_selector(name.into(), &mut env.mem);
    let responds: bool = msg![env; delegate respondsToSelector:sel];
    if let Some((code, message)) = error {
        log!("UIWebView load failed ({}): {}", code, message);
        if responds {
            let domain = ns_string::get_static_str(env, "NSURLErrorDomain");
            let key = ns_string::get_static_str(env, "NSLocalizedDescription");
            let text = ns_string::from_rust_string(env, message);
            let info: id = msg_class![env; NSDictionary dictionaryWithObject:text forKey:key];
            let error: id = msg_class![env; NSError errorWithDomain:domain code:code userInfo:info];
            release(env, text);
            () = msg![env; delegate webView:view didFailLoadWithError:error];
        }
    } else if responds { () = msg![env; delegate webViewDidFinishLoad:view]; }
    release(env, delegate);
}

fn load_page(env: &mut Environment, view: id, page: WebPage, mode: u8) {
    retain(env, view);
    (|| {
        let host = env.objc.borrow_mut::<UIWebViewHostObject>(view);
        host.load_revision = host.load_revision.wrapping_add(1);
        let revision = host.load_revision;
        let delegate = host.delegate;
        retain(env, delegate);
        let request = request_for_url(env, &page.url);
        let sel = env.objc.register_host_selector(
            "webView:shouldStartLoadWithRequest:navigationType:".into(), &mut env.mem);
        let responds: bool = msg![env; delegate respondsToSelector:sel];
        let kind: i32 = match mode { 1 | 2 => 2, 3 => 3, _ => 5 };
        let allowed: bool = !responds || msg![env; delegate webView:view
            shouldStartLoadWithRequest:request navigationType:kind];
        release(env, delegate);
        if env.objc.borrow::<UIWebViewHostObject>(view).load_revision != revision { return; }
        env.objc.borrow_mut::<UIWebViewHostObject>(view).loading = allowed;
        if !allowed { return; }
        let delegate = env.objc.borrow::<UIWebViewHostObject>(view).delegate;
        retain(env, delegate);
        let sel = env.objc.register_host_selector("webViewDidStartLoad:".into(), &mut env.mem);
        let responds: bool = msg![env; delegate respondsToSelector:sel];
        if responds { () = msg![env; delegate webViewDidStartLoad:view]; }
        release(env, delegate);
        if env.objc.borrow::<UIWebViewHostObject>(view).load_revision != revision { return; }
        let frame: CGRect = msg![env; view bounds];
        let result = render_url_to_layer(env, view, &page, frame);
        if env.objc.borrow::<UIWebViewHostObject>(view).load_revision != revision { return; }
        env.objc.borrow_mut::<UIWebViewHostObject>(view).loading = false;
        if let Err(error) = result { notify_load(env, view, Some((-1, error))); return; }
        let url = ns_string::from_rust_string(env, page.url.clone());
        let host = env.objc.borrow_mut::<UIWebViewHostObject>(view);
        let previous = host.current_page.replace(page);
        match mode {
            1 => { host.back_stack.pop(); if let Some(p) = previous { host.forward_stack.push(p); } }
            2 => { host.forward_stack.pop(); if let Some(p) = previous { host.back_stack.push(p); } }
            3 => {}
            _ => { if let Some(p) = previous { host.back_stack.push(p); } host.forward_stack.clear(); }
        }
        let old = std::mem::replace(&mut host.current_url, url);
        release(env, old);
        notify_load(env, view, None);
    })();
    release(env, view);
}

/// Counter used for unique temp filenames.
static SNAP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn page_from_data(env: &mut Environment, bytes: Vec<u8>, mime: String, base: id) -> WebPage {
    let mut url = if base == nil { format!("{}/", env.fs.home_directory().as_str()) }
        else {
            let text: id = msg![env; base absoluteString];
            to_rust_string(env, text).into_owned()
        };
    if url.ends_with('/') { url.push_str("__touchhle_inline__.html"); }
    WebPage { url, body: Some((bytes, mime)) }
}

fn decode_url_path(text: &str) -> Result<String, String> {
    let mut result = Vec::new();
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let a = bytes.next().and_then(|b| (b as char).to_digit(16));
            let b = bytes.next().and_then(|b| (b as char).to_digit(16));
            result.push(match (a, b) {
                (Some(a), Some(b)) => (a * 16 + b) as u8,
                _ => return Err("Malformed URL escape".into()),
            });
        } else { result.push(byte); }
    }
    String::from_utf8(result).map_err(|_| "Invalid UTF-8 URL path".into())
}

fn encode_url_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) { out.push(b as char); }
        else { out.push_str(&format!("%{b:02X}")); }
    }
    out
}

fn local_path(text: &str) -> Result<String, String> {
    let path = if let Some(rest) = text.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        decode_url_path(rest.split(['?', '#']).next().unwrap_or(""))?
    } else { text.to_owned() };
    if !path.starts_with('/') || path.contains(['\\', '\0']) {
        return Err(format!("Unsupported local URL: {text}"));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part { "" | "." => {}, ".." => { parts.pop(); }, _ => parts.push(part) }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn sandbox_path(env: &Environment, path: &str) -> bool {
    let root = env.fs.home_directory().as_str();
    !root.is_empty() && path.strip_prefix(root).is_some_and(|p| p.starts_with('/'))
}

/// Find a headless-capable Chromium binary, including Windows installations.
fn find_chromium_binary() -> Option<PathBuf> {
    // Allow env var override for advanced users / CI.
    if let Ok(path) = std::env::var("TOUCHHLE_CHROMIUM") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(variable) {
            for relative in ["Microsoft/Edge/Application/msedge.exe", "Google/Chrome/Application/chrome.exe"] {
                let path = PathBuf::from(&root).join(relative);
                if path.is_file() { return Some(path); }
            }
        }
    }
    let candidates = [
        "/opt/.devin/chrome/chrome/linux-137.0.7118.2/chrome-linux64/chrome",
        "/opt/.devin/playwright_browsers/chromium-1097/chrome-linux/chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome-stable",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

const MAX_WEB_BYTES: u64 = 16 * 1024 * 1024;

fn web_mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html", "css" => "text/css",
        "js" | "mjs" => "text/javascript", "json" => "application/json",
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif", "webp" => "image/webp", "svg" => "image/svg+xml",
        "woff" => "font/woff", "woff2" => "font/woff2", "ttf" => "font/ttf",
        "mp3" => "audio/mpeg", "mp4" => "video/mp4", "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

fn read_web_file(env: &Environment, path: &str) -> Result<Vec<u8>, String> {
    if !sandbox_path(env, path) { return Err(format!("Outside app sandbox: {path}")); }
    let guest = GuestPath::new(path);
    let size = env.fs.size(guest).map_err(|_| format!("File not found: {path}"))?;
    if size > MAX_WEB_BYTES { return Err(format!("Resource exceeds 16 MiB: {path}")); }
    env.fs.read(guest).map_err(|_| format!("Cannot read: {path}"))
}

fn prepare_page(env: &Environment, page: &WebPage) -> Result<(String, Vec<u8>, String), String> {
    let remote = page.url.starts_with("https://") || page.url.starts_with("http://");
    let mut base = page.url.clone();
    let (mut bytes, mime) = if let Some(body) = &page.body { body.clone() }
    else if remote {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
        let response = agent.get(&page.url).call().map_err(|e| e.to_string())?;
        base = response.get_url().to_owned();
        let mime = response.header("Content-Type").unwrap_or("text/html").to_owned();
        let mut bytes = Vec::new();
        response.into_reader().take(MAX_WEB_BYTES + 1).read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        (bytes, mime)
    } else {
        let path = local_path(&page.url)?;
        (read_web_file(env, &path)?, web_mime(&path).into())
    };
    if bytes.len() as u64 > MAX_WEB_BYTES { return Err("Document exceeds 16 MiB".into()); }
    if mime.contains(['\r', '\n']) { return Err("Invalid content type".into()); }
    let path = if remote {
        if mime.to_ascii_lowercase().starts_with("text/html") {
            let charset = mime.split(';').find_map(|s| s.trim().strip_prefix("charset="));
            let encoding = encoding_rs::Encoding::for_bom(&bytes).map(|(e, _)| e)
                .or_else(|| charset.and_then(|s| encoding_rs::Encoding::for_label(s.trim_matches('"').as_bytes())))
                .unwrap_or(encoding_rs::UTF_8);
            let (html, _, _) = encoding.decode(&bytes);
            let base = base.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
            bytes = format!("<base href=\"{base}\">{html}").into_bytes();
        }
        format!("{}/__touchhle_remote__.html", env.fs.home_directory().as_str())
    } else { local_path(&page.url)? };
    if !sandbox_path(env, &path) { return Err("Document outside app sandbox".into()); }
    let mime = if remote && mime.to_ascii_lowercase().starts_with("text/html") {
        "text/html; charset=utf-8".into()
    } else { mime };
    Ok((path, bytes, mime))
}

/// Snapshot a document and its resources through a temporary loopback server.
fn serve_web_resource(
    env: &Environment, mut stream: TcpStream, authority: &str, prefix: &str,
    main: &(String, Vec<u8>, String),
) -> Result<bool, String> {
    stream.set_read_timeout(Some(Duration::from_millis(200))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(1))).map_err(|e| e.to_string())?;
    let mut request = Vec::new();
    let mut buffer = [0u8; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 || request.len() + n > 16384 { return Err("Invalid HTTP request".into()); }
        request.extend_from_slice(&buffer[..n]);
    }
    let text = String::from_utf8_lossy(&request);
    let mut lines = text.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("");
    let target = first.next().unwrap_or("").split('?').next().unwrap_or("");
    let host = lines.find_map(|line| line.split_once(':')
        .filter(|(key, _)| key.eq_ignore_ascii_case("host")).map(|(_, value)| value.trim()));
    let path = target.strip_prefix(prefix).ok_or("Invalid resource prefix")
        .and_then(|p| decode_url_path(p).map_err(|_| "Invalid resource URL"))
        .and_then(|p| local_path(&p).map_err(|_| "Invalid resource path"));
    let mut is_main = false;
    let resource = if host != Some(authority) || !matches!(method, "GET" | "HEAD") {
        Err("Rejected resource request".to_owned())
    } else {
        path.map_err(str::to_owned).and_then(|path| {
            if path == main.0 {
                is_main = true;
                Ok((main.1.clone(), main.2.clone()))
            } else {
                read_web_file(env, &path).map(|data| (data, web_mime(&path).into()))
            }
        })
    };
    let (status, bytes, mime) = match resource {
        Ok((bytes, mime)) => ("200 OK", bytes, mime),
        Err(error) => {
            log!("UIWebView resource: {}", error);
            is_main = false;
            ("404 Not Found", Vec::new(), "text/plain".into())
        }
    };
    let headers = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n", bytes.len());
    stream.write_all(headers.as_bytes()).map_err(|e| e.to_string())?;
    if method != "HEAD" { stream.write_all(&bytes).map_err(|e| e.to_string())?; }
    Ok(is_main && method == "GET")
}

struct WebTempDirectory(PathBuf);
impl Drop for WebTempDirectory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn snapshot_url_with_chromium(
    env: &Environment, page: &WebPage, width: u32, height: u32,
) -> Result<Vec<u8>, String> {
    let chrome = find_chromium_binary().ok_or("No Chromium browser found; set TOUCHHLE_CHROMIUM")?;
    let main = prepare_page(env, page)?;
    let idx = SNAP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?.as_nanos();
    let name = format!("touchhle_web_{}_{stamp}_{idx}", std::process::id());
    let directory = std::env::temp_dir().join(&name);
    std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let directory = WebTempDirectory(directory);
    let tmp = directory.0.join("snapshot.png");
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let authority = listener.local_addr().map_err(|e| e.to_string())?.to_string();
    let prefix = format!("/{name}");
    let url = format!("http://{authority}{prefix}{}", encode_url_path(&main.0));
    let mut child = Command::new(&chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--hide-scrollbars")
        .arg("--disable-dev-shm-usage")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--virtual-time-budget=1000")
        .arg(format!("--user-data-dir={}", directory.0.join("profile").display()))
        .arg(format!("--window-size={},{}", width, height))
        .arg(format!("--screenshot={}", tmp.display()))
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn().map_err(|e| format!("Cannot start Chromium: {e}"))?;
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut main_served = false;
        loop {
            if Instant::now() >= deadline { return Err("Chromium rendering timed out".into()); }
            for _ in 0..16 {
                match listener.accept() {
                    Ok((stream, _)) => match serve_web_resource(env, stream, &authority, &prefix, &main) {
                        Ok(served) => main_served |= served,
                        Err(error) => log!("UIWebView resource connection: {}", error),
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.to_string()),
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() { return Err(format!("Chromium exited with {status}")); }
                if !main_served { return Err("Chromium did not load the main document".into()); }
                let size = std::fs::metadata(&tmp).map_err(|e| e.to_string())?.len();
                if size > MAX_WEB_BYTES { return Err("Snapshot exceeds 16 MiB".into()); }
                return std::fs::read(&tmp).map_err(|e| e.to_string());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// Snapshot `url` and install the decoded PNG as the UIWebView's
/// `layer.contents` so the user sees the rendered web page.
fn render_url_to_layer(env: &mut Environment, this: id, page: &WebPage, frame: CGRect) -> Result<(), String> {
    if !frame.size.width.is_finite() || !frame.size.height.is_finite()
        || frame.size.width <= 0.0 || frame.size.height <= 0.0
        || frame.size.width > 4096.0 || frame.size.height > 4096.0 {
        return Err("UIWebView has invalid or empty bounds".into());
    }
    let width = frame.size.width.ceil() as u32;
    let height = frame.size.height.ceil() as u32;
    let png = snapshot_url_with_chromium(env, page, width, height)?;
    let image = Image::from_bytes(&png).map_err(|_| "Invalid Chromium PNG snapshot")?;
    let layer: id = msg![env; this layer];
    if layer == nil { return Err("UIWebView has no backing layer".into()); }
    let cg_image = cg_image::from_image(env, image);
    let _: () = msg![env; layer setContents:cg_image];
    cg_image::CGImageRelease(env, cg_image);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_url_round_trip() {
        let path = "/app/校歌/images/song + #1.png";
        assert_eq!(decode_url_path(&encode_url_path(path)).unwrap(), path);
        assert_eq!(decode_url_path("a+b").unwrap(), "a+b");
        for invalid in ["%", "%2", "%GG", "%FF"] {
            assert!(decode_url_path(invalid).is_err());
        }
    }

    #[test]
    fn local_urls_normalize_before_sandbox_check() {
        assert_eq!(local_path("file://localhost/app/pages/../song%20one.html?q=1#top").unwrap(),
            "/app/song one.html");
        assert_eq!(local_path("/app/./pages/../image.png").unwrap(), "/app/image.png");
        assert_eq!(local_path("/app/../../outside").unwrap(), "/outside");
        assert_eq!(local_path("/app/a%20b.html").unwrap(), "/app/a%20b.html");
        for invalid in ["relative.html", "https://example.org/a", "file://server/a",
            "file:///app/%00", "file:///app/%5Csecret"] {
            assert!(local_path(invalid).is_err());
        }
    }

    #[test]
    fn resource_content_types() {
        assert_eq!(web_mime("/app/style.CSS"), "text/css");
        assert_eq!(web_mime("/app/main.js"), "text/javascript");
        assert_eq!(web_mime("/app/image.png"), "image/png");
        assert_eq!(web_mime("/app/font.woff2"), "font/woff2");
    }
}
