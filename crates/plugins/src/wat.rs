//! Small modules in WebAssembly text (WAT) for tests and the authoring guide: the ABI skeleton,
//! a few JSON helpers, and three working modules (a filter that shifts every node, a reporter that
//! draws the bounding box, a check that flags every path). Tests turn the text into binaries with
//! the `wat` crate.
//!
//! The skeleton ([`module`], [`returning`], [`manifest`], [`string`]) is ported from
//! VectorCraft's `crates/plugins/src/wat.rs` (MIT OR Apache-2.0, the same owner; see `NOTICE`)
//! with the exports renamed to `fc_*`.
//!
//! The helpers read JSON as FontCraft writes it (no spaces, keys in field order) and are not
//! general parsers: [`HELPERS`] finds a byte pattern, reads a number (whole part only) and writes
//! a whole number. A key pattern such as `"x":` never matches inside a JSON string, since quotes
//! there are escaped.

/// Where [`returning`] keeps its output in memory (the manifest sits at 16).
const OUTPUT_AT: usize = 0x1_0000;
/// Where the working modules keep their constant strings; their manifests must end before it.
const STRINGS_AT: usize = 0x8000;

/// `s` as the body of a WAT string literal (every byte that is not a letter, digit or space
/// escaped).
pub fn string(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b' ' { char::from(b).to_string() } else { format!("\\{b:02x}") }).collect()
}

/// A module whose `fc_abi_version` returns `abi`, with `manifest` (JSON), an `fc_run` whose body
/// is `run` (parameters `$in $il $pp $pl`, result i64), extra module fields `extra` (imports go
/// here: they come first) and `pages` of initial memory. `fc_alloc` grows memory by whole pages,
/// so its blocks never overlap data segments.
pub fn module_abi(abi: i32, manifest: &str, run: &str, extra: &str, pages: u32) -> String {
    format!(
        r#"(module
  {extra}
  (memory (export "memory") {pages})
  (data (i32.const 16) "{data}")
  (func (export "fc_abi_version") (result i32) (i32.const {abi}))
  (func (export "fc_manifest") (result i64) (i64.or (i64.shl (i64.const {len}) (i64.const 32)) (i64.const 16)))
  (func $alloc (export "fc_alloc") (param $n i32) (result i32) (local $old i32)
    (local.set $old (memory.grow (i32.shr_u (i32.add (local.get $n) (i32.const 65535)) (i32.const 16))))
    (if (result i32) (i32.eq (local.get $old) (i32.const -1)) (then (i32.const 0)) (else (i32.mul (local.get $old) (i32.const 65536)))))
  (func (export "fc_run") (param $in i32) (param $il i32) (param $pp i32) (param $pl i32) (result i64)
    {run}))"#,
        data = string(manifest),
        len = manifest.len(),
    )
}

/// [`module_abi`] for ABI version 1.
pub fn module(manifest: &str, run: &str, extra: &str, pages: u32) -> String {
    module_abi(1, manifest, run, extra, pages)
}

/// An `fc_run` body returning the input unchanged.
pub const ECHO: &str = "(i64.or (i64.shl (i64.extend_i32_u (local.get $il)) (i64.const 32)) (i64.extend_i32_u (local.get $in)))";

/// A module that always returns `output` (whatever it is given).
pub fn returning(manifest: &str, output: &str) -> String {
    let run = format!("(i64.or (i64.shl (i64.const {}) (i64.const 32)) (i64.const {OUTPUT_AT}))", output.len());
    let extra = format!(r#"(data (i32.const {OUTPUT_AT}) "{}")"#, string(output));
    let pages = (OUTPUT_AT + output.len()).div_ceil(0x1_0000) + 1;
    module(manifest, &run, &extra, u32::try_from(pages).unwrap_or(u32::MAX))
}

/// The manifest of a test module `id` of `kind` with `params` (a JSON object of specs).
pub fn manifest(id: &str, kind: &str, params: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"{kind}","params":{params}}}"#)
}

/// Helper functions for `fc_run` bodies:
/// - `$find (start end pattern len) -> i32`: the address of the first match in `start..end`, or -1;
/// - `$num (p) -> i64`: the number at `p` (an optional `-`, digits, an ignored fraction), leaving
///   the address after it in the global `$pos`;
/// - `$int (out v) -> i32`: writes `v` in decimal at `out` and returns the address after it;
/// - `$put (out src len) -> i32`: copies `len` bytes and returns the address after them;
/// - `$pack (ptr len) -> i64`: the `len << 32 | ptr` an `fc_run` returns.
pub const HELPERS: &str = r#"
  (global $pos (mut i32) (i32.const 0))
  (func $find (param $h i32) (param $end i32) (param $pat i32) (param $n i32) (result i32) (local $i i32)
    (block $no (loop $outer
      (br_if $no (i32.gt_u (i32.add (local.get $h) (local.get $n)) (local.get $end)))
      (local.set $i (i32.const 0))
      (block $miss (loop $inner
        (if (i32.ge_u (local.get $i) (local.get $n)) (then (return (local.get $h))))
        (br_if $miss (i32.ne (i32.load8_u (i32.add (local.get $h) (local.get $i))) (i32.load8_u (i32.add (local.get $pat) (local.get $i)))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $inner)))
      (local.set $h (i32.add (local.get $h) (i32.const 1)))
      (br $outer)))
    (i32.const -1))
  (func $num (param $p i32) (result i64) (local $neg i32) (local $v i64) (local $c i32)
    (if (i32.eq (i32.load8_u (local.get $p)) (i32.const 45))
      (then (local.set $neg (i32.const 1)) (local.set $p (i32.add (local.get $p) (i32.const 1)))))
    (block $done (loop $digits
      (local.set $c (i32.sub (i32.load8_u (local.get $p)) (i32.const 48)))
      (br_if $done (i32.gt_u (local.get $c) (i32.const 9)))
      (local.set $v (i64.add (i64.mul (local.get $v) (i64.const 10)) (i64.extend_i32_u (local.get $c))))
      (local.set $p (i32.add (local.get $p) (i32.const 1)))
      (br $digits)))
    (if (i32.eq (i32.load8_u (local.get $p)) (i32.const 46)) (then
      (local.set $p (i32.add (local.get $p) (i32.const 1)))
      (block $fdone (loop $frac
        (br_if $fdone (i32.gt_u (i32.sub (i32.load8_u (local.get $p)) (i32.const 48)) (i32.const 9)))
        (local.set $p (i32.add (local.get $p) (i32.const 1)))
        (br $frac)))))
    (global.set $pos (local.get $p))
    (if (result i64) (local.get $neg) (then (i64.sub (i64.const 0) (local.get $v))) (else (local.get $v))))
  (func $int (param $o i32) (param $v i64) (result i32) (local $a i32) (local $b i32) (local $t i32)
    (if (i64.lt_s (local.get $v) (i64.const 0)) (then
      (i32.store8 (local.get $o) (i32.const 45))
      (local.set $o (i32.add (local.get $o) (i32.const 1)))
      (local.set $v (i64.sub (i64.const 0) (local.get $v)))))
    (local.set $a (local.get $o))
    (loop $digit
      (i32.store8 (local.get $o) (i32.add (i32.const 48) (i32.wrap_i64 (i64.rem_u (local.get $v) (i64.const 10)))))
      (local.set $o (i32.add (local.get $o) (i32.const 1)))
      (local.set $v (i64.div_u (local.get $v) (i64.const 10)))
      (br_if $digit (i64.ne (local.get $v) (i64.const 0))))
    (local.set $b (i32.sub (local.get $o) (i32.const 1)))
    (block $done (loop $swap
      (br_if $done (i32.ge_u (local.get $a) (local.get $b)))
      (local.set $t (i32.load8_u (local.get $a)))
      (i32.store8 (local.get $a) (i32.load8_u (local.get $b)))
      (i32.store8 (local.get $b) (local.get $t))
      (local.set $a (i32.add (local.get $a) (i32.const 1)))
      (local.set $b (i32.sub (local.get $b) (i32.const 1)))
      (br $swap)))
    (local.get $o))
  (func $put (param $o i32) (param $src i32) (param $n i32) (result i32)
    (memory.copy (local.get $o) (local.get $src) (local.get $n))
    (i32.add (local.get $o) (local.get $n)))
  (func $pack (param $ptr i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (local.get $len)) (i64.const 32)) (i64.extend_i32_u (local.get $ptr))))
"#;

/// Constant strings at [`STRINGS_AT`]: a data segment and each string's `(address, length)`.
fn strings(items: &[&str]) -> (String, Vec<(usize, usize)>) {
    let mut at = STRINGS_AT;
    let mut data = String::new();
    let mut places = Vec::new();
    for s in items {
        data.push_str(&format!("(data (i32.const {at}) \"{}\")\n", string(s)));
        places.push((at, s.len()));
        at += s.len();
    }
    (data, places)
}

/// `(address, length)` number `i` of `places` as WAT operands.
fn arg(places: &[(usize, usize)], i: usize) -> String {
    places.get(i).map(|(a, n)| format!("(i32.const {a}) (i32.const {n})")).unwrap_or_default()
}

/// A filter that returns the input's `layer` with every `x` moved by the `dx` parameter and
/// every `y` by `dy` (whole numbers, defaults 10 and 0): nodes and anchors alike.
pub fn shift_filter(id: &str) -> String {
    let man = manifest(id, "filter", r#"{"dx":{"type":"int","default":10},"dy":{"type":"int","default":0}}"#);
    let (data, s) = strings(&[r#""layer":"#, r#""dx":"#, r#""dy":"#, r#"{"error":"no layer"}"#]);
    let (layer, dx, dy, err) = (arg(&s, 0), arg(&s, 1), arg(&s, 2), arg(&s, 3));
    let run = format!(
        r#"(local $p i32) (local $end i32) (local $out i32) (local $o i32) (local $c i32) (local $depth i32) (local $str i32)
    (local $dx i64) (local $dy i64) (local $at i32)
    (local.set $end (i32.add (local.get $in) (local.get $il)))
    (local.set $at (call $find (local.get $pp) (i32.add (local.get $pp) (local.get $pl)) {dx}))
    (if (i32.ge_s (local.get $at) (i32.const 0)) (then (local.set $dx (call $num (i32.add (local.get $at) (i32.const 5))))))
    (local.set $at (call $find (local.get $pp) (i32.add (local.get $pp) (local.get $pl)) {dy}))
    (if (i32.ge_s (local.get $at) (i32.const 0)) (then (local.set $dy (call $num (i32.add (local.get $at) (i32.const 5))))))
    (local.set $p (call $find (local.get $in) (local.get $end) {layer}))
    (if (i32.lt_s (local.get $p) (i32.const 0)) (then (return (call $pack {err}))))
    (local.set $p (i32.add (local.get $p) (i32.const 8)))
    (local.set $out (call $alloc (i32.add (i32.mul (local.get $il) (i32.const 2)) (i32.const 64))))
    (local.set $o (local.get $out))
    (block $done (loop $copy
      (br_if $done (i32.ge_u (local.get $p) (local.get $end)))
      (local.set $c (i32.load8_u (local.get $p)))
      (if (local.get $str)
        (then
          (i32.store8 (local.get $o) (local.get $c))
          (local.set $o (i32.add (local.get $o) (i32.const 1)))
          (local.set $p (i32.add (local.get $p) (i32.const 1)))
          (if (i32.eq (local.get $c) (i32.const 92)) (then
            (i32.store8 (local.get $o) (i32.load8_u (local.get $p)))
            (local.set $o (i32.add (local.get $o) (i32.const 1)))
            (local.set $p (i32.add (local.get $p) (i32.const 1)))))
          (if (i32.eq (local.get $c) (i32.const 34)) (then (local.set $str (i32.const 0)))))
        (else
          (if (i32.and (i32.eq (local.get $c) (i32.const 34))
                       (i32.and (i32.eq (i32.load8_u (i32.add (local.get $p) (i32.const 2))) (i32.const 34))
                                (i32.eq (i32.load8_u (i32.add (local.get $p) (i32.const 3))) (i32.const 58))))
            (then
              (local.set $at (i32.load8_u (i32.add (local.get $p) (i32.const 1))))
              (if (i32.or (i32.eq (local.get $at) (i32.const 120)) (i32.eq (local.get $at) (i32.const 121)))
                (then
                  (local.set $o (call $put (local.get $o) (local.get $p) (i32.const 4)))
                  (local.set $o (call $int (local.get $o)
                    (i64.add (call $num (i32.add (local.get $p) (i32.const 4)))
                      (if (result i64) (i32.eq (local.get $at) (i32.const 120)) (then (local.get $dx)) (else (local.get $dy))))))
                  (local.set $p (global.get $pos))
                  (br $copy)))))
          (if (i32.eq (local.get $c) (i32.const 34)) (then (local.set $str (i32.const 1))))
          (if (i32.or (i32.eq (local.get $c) (i32.const 123)) (i32.eq (local.get $c) (i32.const 91)))
            (then (local.set $depth (i32.add (local.get $depth) (i32.const 1)))))
          (if (i32.or (i32.eq (local.get $c) (i32.const 125)) (i32.eq (local.get $c) (i32.const 93)))
            (then (local.set $depth (i32.sub (local.get $depth) (i32.const 1)))))
          (i32.store8 (local.get $o) (local.get $c))
          (local.set $o (i32.add (local.get $o) (i32.const 1)))
          (local.set $p (i32.add (local.get $p) (i32.const 1)))
          (br_if $done (i32.eqz (local.get $depth)))))
      (br $copy)))
    (call $pack (local.get $out) (i32.sub (local.get $o) (local.get $out)))"#
    );
    module(&man, &run, &format!("{HELPERS}\n{data}"), 1)
}

/// A reporter that draws the bounding box of every `x` and `y` in its input (nodes and anchors)
/// as a red stroked path in the foreground; nothing when there are none.
pub fn bbox_reporter(id: &str) -> String {
    let man = manifest(id, "reporter", "{}");
    let (data, s) = strings(&[
        r#""x":"#,
        r#""y":"#,
        r#"{"foreground":[{"type":"path","commands":[{"op":"moveTo","to":["#,
        r#"]},{"op":"lineTo","to":["#,
        r#"]},{"op":"close"}],"stroke":{"color":[255,0,0,255],"width":1}}]}"#,
        ",",
        "{}",
    ]);
    let (px, py, head, step, tail, comma, empty) = (arg(&s, 0), arg(&s, 1), arg(&s, 2), arg(&s, 3), arg(&s, 4), arg(&s, 5), arg(&s, 6));
    let scan = |pat: &str, lo: &str, hi: &str| {
        format!(
            r#"(local.set $p (local.get $in))
    (block $done (loop $scan
      (local.set $p (call $find (local.get $p) (local.get $end) {pat}))
      (br_if $done (i32.lt_s (local.get $p) (i32.const 0)))
      (local.set $v (call $num (i32.add (local.get $p) (i32.const 4))))
      (local.set $p (global.get $pos))
      (local.set $seen (i32.const 1))
      (if (i64.lt_s (local.get $v) (local.get ${lo})) (then (local.set ${lo} (local.get $v))))
      (if (i64.gt_s (local.get $v) (local.get ${hi})) (then (local.set ${hi} (local.get $v))))
      (br $scan)))"#
        )
    };
    let (sx, sy) = (scan(&px, "x0", "x1"), scan(&py, "y0", "y1"));
    let pt = |x: &str, y: &str| {
        format!(
            "(local.set $o (call $int (local.get $o) (local.get ${x})))
    (local.set $o (call $put (local.get $o) {comma}))
    (local.set $o (call $int (local.get $o) (local.get ${y})))"
        )
    };
    let corners = [pt("x0", "y0"), pt("x1", "y0"), pt("x1", "y1"), pt("x0", "y1")]
        .join(&format!("\n    (local.set $o (call $put (local.get $o) {step}))\n    "));
    let run = format!(
        r#"(local $p i32) (local $end i32) (local $out i32) (local $o i32) (local $seen i32) (local $v i64)
    (local $x0 i64) (local $y0 i64) (local $x1 i64) (local $y1 i64)
    (local.set $end (i32.add (local.get $in) (local.get $il)))
    (local.set $x0 (i64.const 9223372036854775807)) (local.set $y0 (i64.const 9223372036854775807))
    (local.set $x1 (i64.const -9223372036854775808)) (local.set $y1 (i64.const -9223372036854775808))
    {sx}
    {sy}
    (if (i32.eqz (local.get $seen)) (then (return (call $pack {empty}))))
    (local.set $out (call $alloc (i32.const 1024)))
    (local.set $o (call $put (local.get $out) {head}))
    {corners}
    (local.set $o (call $put (local.get $o) {tail}))
    (call $pack (local.get $out) (i32.sub (local.get $o) (local.get $out)))"#
    );
    module(&man, &run, &format!("{HELPERS}\n{data}"), 1)
}

/// A check that reports one warning named `path` for every path in its input (every `"nodes":`
/// key).
pub fn path_check(id: &str) -> String {
    let man = manifest(id, "check", "{}");
    let (data, s) = strings(&[r#""nodes":"#, r#"{"findings":["#, r#"{"name":"path","severity":"warning","message":"a path"}"#, ",", "]}"]);
    let (nodes, head, item, comma, tail) = (arg(&s, 0), arg(&s, 1), arg(&s, 2), arg(&s, 3), arg(&s, 4));
    let run = format!(
        r#"(local $p i32) (local $end i32) (local $n i32) (local $out i32) (local $o i32)
    (local.set $end (i32.add (local.get $in) (local.get $il)))
    (local.set $p (local.get $in))
    (block $done (loop $scan
      (local.set $p (call $find (local.get $p) (local.get $end) {nodes}))
      (br_if $done (i32.lt_s (local.get $p) (i32.const 0)))
      (local.set $n (i32.add (local.get $n) (i32.const 1)))
      (local.set $p (i32.add (local.get $p) (i32.const 8)))
      (br $scan)))
    (local.set $out (call $alloc (i32.add (i32.const 64) (i32.mul (local.get $n) (i32.const 64)))))
    (local.set $o (call $put (local.get $out) {head}))
    (block $end_items (loop $items
      (br_if $end_items (i32.eqz (local.get $n)))
      (local.set $o (call $put (local.get $o) {item}))
      (local.set $n (i32.sub (local.get $n) (i32.const 1)))
      (if (local.get $n) (then (local.set $o (call $put (local.get $o) {comma}))))
      (br $items)))
    (local.set $o (call $put (local.get $o) {tail}))
    (call $pack (local.get $out) (i32.sub (local.get $o) (local.get $out)))"#
    );
    module(&man, &run, &format!("{HELPERS}\n{data}"), 1)
}
