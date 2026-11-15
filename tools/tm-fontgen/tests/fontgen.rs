use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tm_fontgen::charset::Source;
use tm_fontgen::kanji::is_kanji;
use tm_fontgen::{
    FontgenError, Generated, Inputs, build, check, generate, load, proof, raster, report,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn inputs() -> Inputs {
    let r = root();
    load(&r.join("assets/fonts/fontgen.toml"), &r).unwrap()
}

fn real() -> &'static Generated {
    static G: OnceLock<Generated> = OnceLock::new();
    G.get_or_init(|| generate(&inputs()).unwrap())
}

fn face(id: &str) -> &'static raster::Face {
    real().face(id).unwrap()
}

fn string_leaves(t: &toml::Table) -> usize {
    t.values()
        .map(|v| match v {
            toml::Value::Table(t) => string_leaves(t),
            _ => 1,
        })
        .sum()
}

/// ja.toml の文言がすべて、キーの辞書順・重複なしで入っている。
fn assert_all_texts(entries: &[tm_fontgen::text::Entry]) {
    let ja = std::fs::read_to_string(root().join("assets/text/ja.toml")).unwrap();
    let table: toml::Table = toml::from_str(&ja).unwrap();
    assert_eq!(entries.len(), string_leaves(&table));
    assert!(!entries.is_empty());
    assert!(entries.windows(2).all(|w| w[0].key < w[1].key));
}

#[test]
fn real_assets_counts() {
    let g = real();
    assert_eq!(g.checked.chars.len(), 407);
    assert_eq!(g.checked.allowed.joyo.len(), 2136);
    assert_eq!(g.checked.allowed.number_units.len(), 23);
    assert_eq!(g.checked.texts.kanji.len(), 97);
    assert_eq!(g.positions.len(), 812);
    for f in &g.faces {
        assert_eq!(f.glyphs.len(), 407);
        let empty: Vec<char> = f
            .glyphs
            .iter()
            .filter(|g| g.bitmap.is_none())
            .map(|g| g.ch)
            .collect();
        assert_eq!(empty, [' ']);
    }
    let s = g.checked.strip;
    assert_eq!(
        (s.page, s.index, s.y, s.height, s.width, s.render_scale),
        (0, 0, 0, 896, 2048, 3)
    );
    assert!(g.rows_used <= 896);
    assert_eq!((g.image.w, g.image.h), (2048, 896));
    assert!(
        g.glyphs_rs
            .contains("pub const GLYPH_RENDER_SCALE: u32 = 3;")
    );
    for f in &g.faces {
        assert_eq!(f.scale, 3);
        for b in f.glyphs.iter().filter_map(|g| g.bitmap.as_ref()) {
            assert!(b.w % 3 == 0 && b.h % 3 == 0 && b.x_off % 3 == 0 && b.y_off % 3 == 0);
        }
    }
    assert_all_texts(&g.checked.texts.entries);
    assert!(g.glyphs_rs.contains("pub const GLYPH_COUNT: usize = 812;"));
}

#[test]
fn glyph_serial_numbers() {
    let s = &real().glyphs_rs;
    assert!(s.contains(
        "    GlyphEntry { ch: ' ', advance: 6, index: NO_BITMAP, w: 0, h: 0, x_off: 0, y_off: 0 },"
    ));
    assert!(s.contains("GlyphEntry { ch: '!', advance: 6, index: 0, "));
    assert!(s.contains("GlyphEntry { ch: '!', advance: 8, index: 406, "));
    assert!(s.contains("GlyphEntry { ch: '\\'', advance: 6, index: 6, "));
    assert!(s.contains("static BODY12_GLYPHS: [GlyphEntry; 407] = ["));
    assert!(s.contains(
        "pub static FACE_HEAD16: FontFace = FontFace { id: FontId::Head16, size_px: 16, "
    ));
}

#[test]
fn rects_inside_strip_without_overlap() {
    let g = real();
    let bitmaps: Vec<&raster::Bitmap> = g
        .faces
        .iter()
        .flat_map(|f| &f.glyphs)
        .filter_map(|g| g.bitmap.as_ref())
        .collect();
    let mut cover = vec![false; 2048 * 896];
    for (b, &(x, y)) in bitmaps.iter().zip(&g.positions) {
        assert!(x + b.w <= 2048 && y + b.h <= 896);
        for yy in y..y + b.h {
            for xx in x..x + b.w {
                let i = (yy * 2048 + xx) as usize;
                assert!(!cover[i], "重なり ({xx}, {yy})");
                cover[i] = true;
            }
        }
    }
}

#[test]
fn every_required_char_has_a_bitmap_in_both_faces() {
    let mut required: Vec<char> = tm_numeric::OUTPUT_CHARS.chars().collect();
    for (u, y) in tm_numeric::JA_UNITS {
        required.extend(u.chars().chain(y.chars()));
    }
    required.extend(&real().checked.texts.kanji);
    for id in ["Body12", "Head16"] {
        for &c in &required {
            let g = face(id)
                .glyph(c)
                .unwrap_or_else(|| panic!("{id} に {c} がない"));
            assert!(g.bitmap.is_some(), "{id} の {c} の画素がない");
        }
    }
}

#[test]
fn sizhi_u79ed_has_a_glyph() {
    let i = inputs();
    let font = raster::load_font(&i.font, 16, "f").unwrap();
    assert_ne!(font.lookup_glyph_index('\u{79ED}'), 0);
    assert!(face("Body12").glyph('秭').unwrap().bitmap.is_some());
    assert!(face("Head16").glyph('秭').unwrap().bitmap.is_some());
}

/// フォントの字 = [charset] ∪ ja.toml の漢字 ∪ tm-numeric の字(15 §4.3)。
#[test]
fn font_chars_are_charset_text_kanji_and_numeric() {
    let g = real();
    let charset = inputs().config.charset_chars("f").unwrap();
    let mut want: std::collections::BTreeSet<char> = charset.clone();
    want.extend(&g.checked.texts.kanji);
    want.extend(tm_fontgen::charset::numeric_chars());
    let got: std::collections::BTreeSet<char> = g.checked.chars.keys().copied().collect();
    assert_eq!(got, want);
    let kanji: Vec<char> = got.iter().copied().filter(|&c| is_kanji(c)).collect();
    assert_eq!(kanji.len(), 122);
    assert!(!got.contains(&'丁') && !got.contains(&'港'));
    assert_eq!(g.checked.chars[&'電'], Source::Text);
    assert_eq!(g.checked.chars[&'秭'], Source::Numeric);
    assert_eq!(g.checked.chars[&'あ'], Source::Charset);
}

fn with_text(extra: &str) -> Inputs {
    let mut i = inputs();
    i.ja.push_str(&format!("\n[zz_test]\nx = \"{extra}\"\n"));
    i
}

#[test]
fn new_kanji_in_ja_toml_enters_the_font() {
    let g = generate(&with_text("港と峠{unit}")).unwrap();
    assert_eq!(g.checked.chars[&'港'], Source::Text);
    assert_eq!(g.checked.chars[&'峠'], Source::Text);
    assert_eq!(g.checked.chars.len(), real().checked.chars.len() + 2);
    assert_eq!(g.positions.len(), real().positions.len() + 4);
    assert_eq!(
        report::non_kyoiku(&g),
        Some(['峠', '戻', '抜', '換', '枠'].into())
    );
    let r = report::report(&g);
    assert!(r.contains("教育漢字でない字: 5\n  峠戻抜換枠\n"), "{r}");
}

#[test]
fn text_kanji_errors() {
    assert_eq!(check(&with_text("繋ぐ")).unwrap_err().code(), "FG-E06");
    assert_eq!(check(&with_text("叱る")).unwrap_err().code(), "FG-E06");
    assert_eq!(check(&with_text("阿の字")).unwrap_err().code(), "FG-E07");
    assert_eq!(check(&with_text("祇")).unwrap_err().code(), "FG-E07");
    assert_eq!(
        check(&with_text("すごい\u{FF01}")).unwrap_err().code(),
        "FG-E05"
    );
    assert_eq!(
        check(&with_text("あ\u{3000}い")).unwrap_err().code(),
        "FG-E05"
    );
    for ok in ["量が多い", "他の駅", "1垓円", "1阿僧祇円", "𠮟る"] {
        assert!(check(&with_text(ok)).is_ok(), "{ok}");
    }
}

#[test]
fn allowed_kanji_format_error_is_e03() {
    let mut i = inputs();
    i.allowed_kanji = i.allowed_kanji.replacen('𠮟', "", 1);
    assert_eq!(check(&i).unwrap_err().code(), "FG-E03");
}

#[test]
fn allowed_list_kanji_without_glyph_is_e10() {
    let font = raster::load_font(&inputs().font, 16, "f").unwrap();
    let none = (0x20000..0x2A6E0)
        .filter_map(char::from_u32)
        .find(|&c| font.lookup_glyph_index(c) == 0)
        .unwrap();
    let mut i = inputs();
    i.allowed_kanji = i.allowed_kanji.replacen('𠮟', &none.to_string(), 1);
    assert_eq!(
        check(&i).unwrap_err(),
        FontgenError::MissingGlyph {
            ch: none,
            source: "[joyo_2010]"
        }
    );
}

#[test]
fn shikaru_u20b9f_and_all_allowed_kanji_have_glyphs() {
    let font = raster::load_font(&inputs().font, 16, "f").unwrap();
    raster::check_glyphs(
        &font,
        &tm_fontgen::charset::allowed_chars(&real().checked.allowed),
    )
    .unwrap();
    assert_ne!(font.lookup_glyph_index('\u{20B9F}'), 0);
}

#[test]
fn strip_usage_over_the_threshold_warns() {
    let used = real().rows_used;
    let warns = |height: u32| {
        let mut i = inputs();
        i.atlas = format!(
            "width = 2048\nrender_scale = 3\n[[page]]\nstrips = [ {{ owner = \"font\", height = {height} }} ]\n"
        );
        let g = generate(&i).unwrap();
        assert_eq!(g.rows_used, used);
        g.warnings.iter().any(|w| w.contains("80%"))
    };
    assert!(warns(used * 100 / 80 - 1));
    assert!(!warns(used * 100 / 80 + 1));
    assert!(!real().warnings.iter().any(|w| w.contains("80%")));
    assert!(real().warnings.len() >= real().checked.texts.warnings.len());
}

#[test]
fn contrast_applies_per_face() {
    let mut i = inputs();
    assert_eq!(
        (i.config.face[0].contrast, i.config.face[1].contrast),
        (30, 0)
    );
    i.config.face[0].contrast = 0;
    let g = generate(&i).unwrap();
    let table = raster::contrast_table(30);
    let (base, tuned) = (&g.faces, &real().faces);
    for (b, t) in base[0].glyphs.iter().zip(&tuned[0].glyphs) {
        if let (Some(b), Some(t)) = (&b.bitmap, &t.bitmap) {
            let want: Vec<u8> = b.alpha.iter().map(|&a| table[a as usize]).collect();
            assert_eq!(t.alpha, want);
        }
    }
    assert_ne!(base[0], tuned[0]);
    assert_eq!(base[1], tuned[1]);
}

#[test]
fn proof_images() {
    let g = real();
    let images = proof::proof(g, 1);
    let names: Vec<&str> = images.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names.last(), Some(&"kanji.png"));
    for f in &g.faces {
        let pages: Vec<&tm_fontgen::emit::Image> = images
            .iter()
            .filter(|(n, _)| n.starts_with(&format!("text_{}_", f.def.id)))
            .map(|(_, img)| img)
            .collect();
        assert!(names.contains(&format!("text_{}_01.png", f.def.id).as_str()));
        let lines: usize = g
            .checked
            .texts
            .entries
            .iter()
            .map(|e| 1 + e.value.split('\n').count())
            .sum();
        let per_page =
            ((proof::PAGE.1 - 2 * proof::MARGIN) / u32::from(f.def.line_height)) as usize;
        assert!(pages.len() >= lines.div_ceil(per_page));
        assert!(
            pages
                .iter()
                .all(|p| (p.w, p.h) == (proof::PAGE.0 * 3, proof::PAGE.1 * 3))
        );
    }
    let sheet = &images.last().unwrap().1;
    let lit = sheet
        .px
        .iter()
        .filter(|p| p[0] == 0xFF && p[1] == 0xFF)
        .count();
    assert!(lit > 0);
    assert_eq!((images[0].1.w, images[0].1.h), (1920, 1080), "描画の解像度");
    let x2 = proof::upscale(&images[0].1, 2);
    assert_eq!(x2.get(3, 5), images[0].1.get(1, 2));
    assert_eq!(proof::proof(g, 1), images);
}

#[test]
fn strip_warning_boundary() {
    let strip = tm_fontgen::config::Strip {
        width: 1024,
        page: 0,
        index: 0,
        y: 0,
        height: 100,
        render_scale: 1,
    };
    assert_eq!(tm_fontgen::strip_warning(80, &strip, 80), None);
    assert!(tm_fontgen::strip_warning(81, &strip, 80).is_some());
    assert_eq!(tm_fontgen::strip_warning(50, &strip, 50), None);
}

fn page_has_line(page: &tm_fontgen::emit::Image, face: &raster::Face, line: &str) -> bool {
    let img = tm_fontgen::preview(face, line, 1);
    let k = face.scale;
    let (lh, margin) = (u32::from(face.def.line_height) * k, proof::MARGIN * k);
    let w = img.w.min(page.w - margin);
    (0..(page.h - margin) / lh).any(|i| {
        let top = margin + i * lh;
        (0..lh).all(|y| (0..w).all(|x| page.get(margin + x, top + y) == img.get(x, y)))
    })
}

#[test]
fn proof_pages_show_every_text_line() {
    let g = real();
    let images = proof::proof(g, 1);
    for f in &g.faces {
        let pages: Vec<&tm_fontgen::emit::Image> = images
            .iter()
            .filter(|(n, _)| n.starts_with(&format!("text_{}_", f.def.id)))
            .map(|(_, img)| img)
            .collect();
        for e in &g.checked.texts.entries {
            for line in e.value.split('\n') {
                if line.is_empty() {
                    continue;
                }
                let found = pages.iter().any(|p| page_has_line(p, f, line));
                assert!(found, "{} の {line:?} が頁にない", f.def.id);
            }
        }
        let last = g.checked.texts.entries.last().unwrap();
        let line = last.value.split('\n').next_back().unwrap();
        assert!(
            page_has_line(pages.last().unwrap(), f, line),
            "{}",
            f.def.id
        );
    }
}

#[test]
fn proof_kanji_sheet_lists_every_font_kanji() {
    let g = real();
    let images = proof::proof(g, 1);
    let kanji: Vec<char> = g
        .checked
        .chars
        .keys()
        .copied()
        .filter(|&c| is_kanji(c))
        .collect();
    assert!(kanji.contains(&'秭') && kanji.contains(&'電'));
    let sheet = &images.iter().find(|(n, _)| n == "kanji.png").unwrap().1;
    assert_eq!(*sheet, proof::kanji_sheet(&g.faces, &kanji, 1));
    let text_only: Vec<char> = g.checked.texts.kanji.iter().copied().collect();
    assert_ne!(*sheet, proof::kanji_sheet(&g.faces, &text_only, 1));
}

#[test]
fn missing_glyph_is_e10() {
    let mut i = inputs();
    i.config.charset.chars.push('\u{E000}');
    let e = check(&i).unwrap_err();
    assert_eq!(
        e,
        FontgenError::MissingGlyph {
            ch: '\u{E000}',
            source: "[charset]"
        }
    );
    assert!(e.to_string().contains("U+E000"));
}

#[test]
fn font_hash_mismatch_is_e02() {
    let mut i = inputs();
    i.font.push(0);
    assert_eq!(generate(&i).unwrap_err().code(), "FG-E02");
}

#[test]
fn strip_overflow_is_e11() {
    let mut i = inputs();
    let height = real().rows_used - 1;
    i.atlas = format!(
        "width = 2048\nrender_scale = 3\n[[page]]\nstrips = [ {{ owner = \"font\", height = {height} }} ]\n"
    );
    assert_eq!(generate(&i).unwrap_err().code(), "FG-E11");
    assert_eq!(check(&i).unwrap_err().code(), "FG-E11");
}

#[test]
fn check_detects_raster_errors() {
    let mut i = inputs();
    i.config.face[0].line_height = 12;
    let e = check(&i).unwrap_err();
    assert_eq!(e.code(), "FG-E01");
    assert!(e.to_string().contains("line_height"), "{e}");
    let mut i = inputs();
    i.config.face[1].size_px = 200;
    i.config.face[1].line_height = 255;
    let e = check(&i).unwrap_err();
    assert_eq!(e.code(), "FG-E01");
    assert!(e.to_string().contains("i8"), "{e}");
    assert_all_texts(&check(&inputs()).unwrap().texts.entries);
}

fn block<'a>(src: &'a str, header: &str) -> Vec<&'a str> {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("{header} がない"));
    src[start..]
        .lines()
        .skip(1)
        .take_while(|l| *l != "];")
        .collect()
}

fn entry_char(line: &str) -> char {
    let lit = line
        .split("ch: ")
        .nth(1)
        .unwrap()
        .split(", advance")
        .next()
        .unwrap();
    match lit {
        "'\\''" => '\'',
        "'\\\\'" => '\\',
        _ => lit.trim_matches('\'').chars().next().unwrap(),
    }
}

#[test]
fn ascii_table_and_full_advance_in_glyphs_rs() {
    let s = &real().glyphs_rs;
    for (upper, id, size) in [("BODY12", "Body12", 12), ("HEAD16", "Head16", 16)] {
        let entries: Vec<char> = block(s, &format!("static {upper}_GLYPHS: "))
            .iter()
            .map(|l| entry_char(l))
            .collect();
        let expected: Vec<char> = face(id).glyphs.iter().map(|g| g.ch).collect();
        assert_eq!(entries, expected, "{upper}_GLYPHS の字の並び");
        let ascii = block(s, &format!("static {upper}_ASCII: "));
        assert_eq!(ascii.len(), 95);
        for (k, line) in ascii.iter().enumerate() {
            let idx: usize = line.trim().trim_end_matches(',').parse().unwrap();
            assert_eq!(entries[idx] as u32, 0x20 + k as u32, "{upper}_ASCII[{k}]");
        }
        let face_line = s
            .lines()
            .find(|l| l.starts_with(&format!("pub static FACE_{upper}: ")))
            .unwrap();
        let full = face(id).glyph('あ').unwrap().advance;
        assert_eq!(full, size);
        assert!(
            face_line.contains(&format!("size_px: {size}, ")),
            "{face_line}"
        );
        assert!(
            face_line.contains(&format!("full_advance: {full}, ")),
            "{face_line}"
        );
    }
}

#[test]
fn unreadable_config_is_e01() {
    let r = root();
    assert_eq!(
        load(&r.join("assets/fonts/none.toml"), &r)
            .unwrap_err()
            .code(),
        "FG-E01"
    );
    assert_eq!(
        tm_fontgen::config::Config::parse("font = 1", "fontgen.toml")
            .unwrap_err()
            .code(),
        "FG-E01"
    );
}

#[test]
fn half_and_full_advances() {
    for (id, half, full) in [("Body12", 6, 12), ("Head16", 8, 16)] {
        let f = face(id);
        for g in &f.glyphs {
            let want = if (' '..='~').contains(&g.ch) {
                half
            } else {
                full
            };
            assert_eq!(g.advance, want, "{id} {:?}", g.ch);
        }
        assert!(f.ascent + f.descent <= u32::from(f.def.line_height));
    }
    assert_eq!(face("Body12").glyph('あ').unwrap().advance, 12);
    assert_eq!(face("Body12").glyph('A').unwrap().advance, 6);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn bitmap_golden_sha256() {
    let f = face("Body12");
    let digest = |c: char| {
        let b = f.glyph(c).unwrap().bitmap.as_ref().unwrap();
        let mut bytes = vec![b.w as u8, b.h as u8, b.x_off as u8, b.y_off as u8];
        bytes.extend(&b.alpha);
        tm_fontgen::sha256_hex(&bytes)
    };
    assert_eq!(
        digest('電'),
        "5ab09153bf9a3004d0c45797a530327c73827eb24b7f02bb824871229bd4565b"
    );
    assert_eq!(
        digest('あ'),
        "003bc12fb9dee41ab885a3a0a75ec79fe3b337d4748eba3ab36a301bee791e9b"
    );
    assert_eq!(
        digest('A'),
        "8ed3a9953fc283455ee72b5e27d5ad865f3e07427b4b25b484ec591165e2d14d"
    );
}

fn read_all(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .map(|e| {
            (
                e.file_name().into_string().unwrap(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn build_twice_is_byte_identical() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tm-fontgen-determinism");
    let _ = std::fs::remove_dir_all(&base);
    let i = inputs();
    build(&i, &base.join("a")).unwrap();
    build(&i, &base.join("b")).unwrap();
    let (a, b) = (read_all(&base.join("a")), read_all(&base.join("b")));
    let names: Vec<&str> = a.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names, ["atlas_p0_s0_font.png", "glyphs.rs", "texts.rs"]);
    assert!(a == b, "2 回の生成でバイト列が違う");
    std::fs::remove_dir_all(&base).unwrap();
}

fn committed(name: &str) -> Vec<u8> {
    std::fs::read(root().join("crates/tm-assets/generated").join(name)).unwrap()
}

#[test]
fn committed_texts_rs_is_up_to_date() {
    assert!(committed("texts.rs") == real().checked.texts_rs.as_bytes());
}

#[cfg(target_arch = "x86_64")]
#[test]
fn committed_atlas_and_glyphs_are_up_to_date() {
    assert!(committed("glyphs.rs") == real().glyphs_rs.as_bytes());
    assert!(committed("atlas_p0_s0_font.png") == real().png);
}

#[test]
fn png_has_only_required_chunks() {
    let png = &real().png;
    let mut p = 8;
    let mut kinds = Vec::new();
    while p < png.len() {
        let n = u32::from_be_bytes(png[p..p + 4].try_into().unwrap()) as usize;
        kinds.push(String::from_utf8(png[p + 4..p + 8].to_vec()).unwrap());
        p += 12 + n;
    }
    assert_eq!(kinds.first().map(String::as_str), Some("IHDR"));
    assert_eq!(kinds.last().map(String::as_str), Some("IEND"));
    assert!(
        kinds
            .iter()
            .all(|k| matches!(k.as_str(), "IHDR" | "IDAT" | "IEND"))
    );
    assert_eq!(&png[16..26], &[0, 0, 8, 0, 0, 0, 3, 0x80, 8, 6]);
}

/// `alt_keys` の文言の字は、元の字形の枠をちょうど 2 倍にした大きい版を持ち、ほかの字は持たない。知らないキーは FG-E01。
#[test]
fn alt_keys_give_their_chars_a_twice_sized_bitmap() {
    let g = real();
    let head = g.face("Head16").unwrap();
    let alts: String = head
        .glyphs
        .iter()
        .filter(|gl| gl.alt.is_some())
        .map(|gl| gl.ch)
        .collect();
    assert_eq!(alts, "!1円大数無量");
    for gl in head.glyphs.iter().filter(|gl| gl.alt.is_some()) {
        let (b, a) = (gl.bitmap.as_ref().unwrap(), gl.alt.as_ref().unwrap());
        assert_eq!(
            (a.w, a.h, a.x_off, a.y_off),
            (2 * b.w, 2 * b.h, 2 * b.x_off, 2 * b.y_off),
            "{}",
            gl.ch
        );
        assert!(
            a.alpha.iter().filter(|&&v| v == 255).count()
                > 3 * b.alpha.iter().filter(|&&v| v == 255).count()
        );
    }
    assert!(
        g.face("Body12")
            .unwrap()
            .glyphs
            .iter()
            .all(|gl| gl.alt.is_none())
    );
    assert_eq!(g.alt_positions.len(), 7);
    assert!(
        g.glyphs_rs
            .contains("pub const GLYPH_ALT_COUNT: usize = 7;")
    );
    let mut i = inputs();
    i.config.face[1].alt_keys = vec!["no.such.key".into()];
    assert_eq!(generate(&i).unwrap_err().code(), "FG-E01");
}
