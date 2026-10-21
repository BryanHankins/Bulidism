//! File exports with no third-party dependencies: XLSX takeoff workbook and BCF 2.1
//! clash topics, both written through a small stored-only ZIP writer. Output is
//! deterministic (fixed timestamps, GUIDs derived from element ids), so the same
//! project always produces the same bytes.
use bim_core::*;
use bim_geom::Clash;
use std::fmt::Write as _;
use uuid::Uuid;

pub mod zip;

/// Fixed creation date stamped on BCF topics (the ZIP entries use the DOS epoch).
pub const CREATION_DATE: &str = "2026-01-01T00:00:00Z";
const AUTHOR: &str = "buildsim";

/// XML 1.0 text/attribute escape; drops characters XML 1.0 cannot carry.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\t' | '\n' | '\r' => o.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => o.push(c),
        }
    }
    o
}

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// One spreadsheet cell. Strings are inline (never evaluated as formulas, so no
/// CSV-style `'` defusing is needed); non-finite numbers become empty cells.
enum Cell {
    Str(String),
    Num(f64),
}
/// Style indexes into the `cellXfs` written by `styles_xml`.
const S_HEAD: u8 = 1;
const S_QTY: u8 = 2;
const S_USD: u8 = 3;
const S_TOTAL_USD: u8 = 4;

fn col(i: usize) -> char {
    (b'A' + i as u8) as char
}

/// Worksheet XML: frozen header row, column widths, rows of (cell, style).
fn sheet_xml(widths: &[f32], rows: &[Vec<(Cell, u8)>]) -> String {
    let mut x = String::with_capacity(rows.len() * 200 + 512);
    x.push_str(XML_DECL);
    let _ = write!(x, "<worksheet xmlns=\"{NS_MAIN}\"><sheetViews><sheetView workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/></sheetView></sheetViews><cols>");
    for (i, w) in widths.iter().enumerate() {
        let _ = write!(
            x,
            "<col min=\"{n}\" max=\"{n}\" width=\"{w}\" customWidth=\"1\"/>",
            n = i + 1
        );
    }
    x.push_str("</cols><sheetData>");
    for (r, row) in rows.iter().enumerate() {
        let _ = write!(x, "<row r=\"{}\">", r + 1);
        for (c, (cell, s)) in row.iter().enumerate() {
            let at = format!("{}{}", col(c), r + 1);
            let st = if *s == 0 {
                String::new()
            } else {
                format!(" s=\"{s}\"")
            };
            match cell {
                Cell::Str(v) => {
                    let _ = write!(
                        x,
                        "<c r=\"{at}\"{st} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                        esc(v)
                    );
                }
                Cell::Num(v) if v.is_finite() => {
                    let _ = write!(x, "<c r=\"{at}\"{st}><v>{v}</v></c>");
                }
                Cell::Num(_) => {}
            }
        }
        x.push_str("</row>");
    }
    x.push_str("</sheetData></worksheet>");
    x
}

fn styles_xml() -> String {
    format!(
        "{XML_DECL}<styleSheet xmlns=\"{NS_MAIN}\">\
<numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"0.000\"/></numFmts>\
<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font><font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
<fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
<cellXfs count=\"5\">\
<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
<xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/>\
<xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
<xf numFmtId=\"4\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
<xf numFmtId=\"4\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\" applyNumberFormat=\"1\"/>\
</cellXfs>\
<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>"
    )
}

fn header(names: &[&str]) -> Vec<(Cell, u8)> {
    names
        .iter()
        .map(|n| (Cell::Str(n.to_string()), S_HEAD))
        .collect()
}

/// XLSX workbook: sheet "Takeoff" (the CSV columns, numbers as numbers) and sheet
/// "By phase" (schedule days, element count and cost per phase, plus a total row).
pub fn xlsx(project: &Project) -> Vec<u8> {
    let lines = bim_takeoff::takeoff(project);
    let mut takeoff = vec![header(&[
        "element",
        "cost_code",
        "phase",
        "qty",
        "unit",
        "cost_usd",
        "lod",
    ])];
    for l in &lines {
        takeoff.push(vec![
            (Cell::Str(l.name.clone()), 0),
            (Cell::Str(l.cost_code.clone()), 0),
            (Cell::Num(l.phase.0 as f64), 0),
            // same rounding as the CSV's {:.3}
            (Cell::Num((l.qty as f64 * 1000.0).round() / 1000.0), S_QTY),
            (Cell::Str(format!("{:?}", l.unit)), 0),
            (Cell::Num(l.cost.dollars()), S_USD),
            (Cell::Str(format!("{:?}", l.lod)), 0),
        ]);
    }
    let by_cost = bim_takeoff::cost_by_phase(&lines);
    let sched = bim_schedule::schedule(&project.phases).ok();
    let mut phases: Vec<&Phase> = project.phases.iter().collect();
    phases.sort_by_key(|p| p.id);
    let mut by_phase = vec![header(&[
        "phase",
        "name",
        "start_day",
        "end_day",
        "elements",
        "cost_usd",
    ])];
    let day = |f: fn(&bim_schedule::ScheduledPhase) -> u32, id| {
        sched
            .as_ref()
            .and_then(|s| s.phase(id))
            .map_or(f64::NAN, |p| f(p) as f64)
    };
    for ph in &phases {
        by_phase.push(vec![
            (Cell::Num(ph.id.0 as f64), 0),
            (Cell::Str(ph.name.clone()), 0),
            (Cell::Num(day(|p| p.start_day, ph.id)), 0),
            (Cell::Num(day(|p| p.end_day, ph.id)), 0),
            (
                Cell::Num(lines.iter().filter(|l| l.phase == ph.id).count() as f64),
                0,
            ),
            (
                Cell::Num(by_cost.get(&ph.id).copied().unwrap_or_default().dollars()),
                S_USD,
            ),
        ]);
    }
    by_phase.push(vec![
        (Cell::Str("TOTAL".into()), S_HEAD),
        (Cell::Str(String::new()), 0),
        (Cell::Num(f64::NAN), 0),
        (
            Cell::Num(sched.as_ref().map_or(f64::NAN, |s| s.total_days as f64)),
            S_HEAD,
        ),
        (Cell::Num(lines.len() as f64), S_HEAD),
        (
            Cell::Num(lines.iter().map(|l| l.cost).sum::<Cents>().dollars()),
            S_TOTAL_USD,
        ),
    ]);

    let sheets = ["Takeoff", "By phase"];
    let mut z = zip::ZipWriter::new();
    let mut ct = format!("{XML_DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    let mut wb = format!("{XML_DECL}<workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_REL}\"><sheets>");
    let mut rels = format!("{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\">");
    for (i, name) in sheets.iter().enumerate() {
        let n = i + 1;
        let _ = write!(ct, "<Override PartName=\"/xl/worksheets/sheet{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>");
        let _ = write!(
            wb,
            "<sheet name=\"{}\" sheetId=\"{n}\" r:id=\"rId{n}\"/>",
            esc(name)
        );
        let _ = write!(rels, "<Relationship Id=\"rId{n}\" Type=\"{NS_REL}/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>");
    }
    ct.push_str("</Types>");
    wb.push_str("</sheets></workbook>");
    let _ = write!(
        rels,
        "<Relationship Id=\"rId{}\" Type=\"{NS_REL}/styles\" Target=\"styles.xml\"/></Relationships>",
        sheets.len() + 1
    );
    z.add("[Content_Types].xml", ct.as_bytes());
    z.add(
        "_rels/.rels",
        format!("{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{NS_REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>").as_bytes(),
    );
    z.add("xl/workbook.xml", wb.as_bytes());
    z.add("xl/_rels/workbook.xml.rels", rels.as_bytes());
    z.add("xl/styles.xml", styles_xml().as_bytes());
    z.add(
        "xl/worksheets/sheet1.xml",
        sheet_xml(&[28.0, 11.0, 7.0, 11.0, 6.0, 13.0, 8.0], &takeoff).as_bytes(),
    );
    z.add(
        "xl/worksheets/sheet2.xml",
        sheet_xml(&[7.0, 22.0, 10.0, 10.0, 10.0, 14.0], &by_phase).as_bytes(),
    );
    z.finish()
}

/// IFC compressed GlobalId (22 chars) for a UUID; inverse of `bim_ifc::guid_to_uuid`.
pub fn ifc_guid(u: Uuid) -> String {
    const B64: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";
    let n = u.as_u128();
    (0..22)
        .map(|i| B64[((n >> (6 * (21 - i))) & 63) as usize] as char)
        .collect()
}

/// Deterministic UUID (version 8, "custom") from a tag and some bytes: two FNV-1a 64 hashes.
fn derived_uuid(tag: &str, parts: &[&[u8]]) -> Uuid {
    let fnv = |seed: u64| {
        let mut h = seed;
        for b in tag
            .bytes()
            .chain(parts.iter().flat_map(|p| p.iter().copied()))
        {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        h
    };
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&fnv(0xcbf2_9ce4_8422_2325).to_be_bytes());
    b[8..].copy_from_slice(&fnv(0x84222325_cbf29ce4).to_be_bytes());
    b[6] = (b[6] & 0x0f) | 0x80;
    b[8] = (b[8] & 0x3f) | 0x80;
    Uuid::from_bytes(b)
}

/// Model (Y up) -> IFC/BCF world (Z up): (x, y, z) -> (x, -z, y), as bim-ifc imports inversely.
fn to_ifc(v: glam::Vec3) -> glam::Vec3 {
    glam::vec3(v.x, -v.z, v.y)
}

fn xyz(tag: &str, v: glam::Vec3) -> String {
    format!("<{tag}><X>{}</X><Y>{}</Y><Z>{}</Z></{tag}>", v.x, v.y, v.z)
}

/// Topic GUID for a clash, from the two element ids (stable across runs and exports).
pub fn clash_topic_guid(c: &Clash) -> Uuid {
    derived_uuid("bcf-topic", &[c.a.0.as_bytes(), c.b.0.as_bytes()])
}

/// BCF 2.1 archive (.bcfzip): `bcf.version`, `project.bcfp`, and one folder per clash
/// (topic GUID) holding `markup.bcf` and `viewpoint.bcfv`. The viewpoint selects both
/// elements and frames the clash region with a perspective camera (IFC Z-up coordinates).
///
/// Component `IfcGuid` is the element UUID in IFC's 22-character compressed form, and
/// `AuthoringToolId` carries the plain UUID. For elements imported from IFC this is the
/// original IfcGlobalId (bim-ifc derives the UUID from it); for elements authored here it
/// only matches an IFC file once an IFC export writes the same UUIDs as GlobalIds.
pub fn bcf(project: &Project, clashes: &[Clash]) -> Vec<u8> {
    let ix = ProjectIndex::new(project);
    let mut z = zip::ZipWriter::new();
    z.add(
        "bcf.version",
        format!("{XML_DECL}<Version VersionId=\"2.1\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"version.xsd\"><DetailedVersion>2.1</DetailedVersion></Version>").as_bytes(),
    );
    z.add(
        "project.bcfp",
        format!(
            "{XML_DECL}<ProjectExtension xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"project.xsd\"><Project ProjectId=\"{}\"><Name>{}</Name></Project><ExtensionSchema></ExtensionSchema></ProjectExtension>",
            derived_uuid("bcf-project", &[project.name.as_bytes()]),
            esc(&project.name)
        )
        .as_bytes(),
    );
    let label = |id: ElementId| {
        ix.get(id).map_or_else(
            |_| format!("{}", id.0),
            |e| format!("{} '{}'", format_args!("{:?}", e.discipline), e.name),
        )
    };
    for c in clashes {
        let topic = clash_topic_guid(c);
        let vp = derived_uuid("bcf-viewpoint", &[topic.as_bytes()]);
        let (la, lb) = (label(c.a), label(c.b));
        let (lo, hi) = (c.region.min, c.region.max);
        let markup = format!(
            "{XML_DECL}<Markup xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"markup.xsd\">\
<Topic Guid=\"{topic}\" TopicType=\"Clash\" TopicStatus=\"Open\">\
<Title>Clash: {}</Title><Labels>Clash</Labels>\
<CreationDate>{CREATION_DATE}</CreationDate><CreationAuthor>{AUTHOR}</CreationAuthor>\
<Description>{}</Description></Topic>\
<Viewpoints Guid=\"{vp}\"><Viewpoint>viewpoint.bcfv</Viewpoint></Viewpoints></Markup>",
            esc(&format!("{la} x {lb}")),
            esc(&format!(
                "{la} overlaps {lb} by {:.3} m3. Overlap region (model metres, Y up): \
                 [{:.3}, {:.3}, {:.3}] to [{:.3}, {:.3}, {:.3}].",
                c.overlap_m3, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
            )),
        );
        // camera on the box diagonal, above and to the side, framing the region
        let (a, b) = (to_ifc(lo), to_ifc(hi));
        let center = (a + b) * 0.5;
        let radius = ((a - b).length() * 0.5).max(0.5);
        let fov = 60.0f32;
        let dist = radius / (fov.to_radians() * 0.5).tan() * 1.5;
        let dir = glam::vec3(1.0, 1.0, -1.0).normalize();
        let up = (glam::Vec3::Z - dir * dir.z).normalize();
        let mut sel = String::new();
        for id in [c.a, c.b] {
            let _ = write!(
                sel,
                "<Component IfcGuid=\"{}\"><OriginatingSystem>{AUTHOR}</OriginatingSystem><AuthoringToolId>{}</AuthoringToolId></Component>",
                ifc_guid(id.0),
                id.0
            );
        }
        let viewpoint = format!(
            "{XML_DECL}<VisualizationInfo Guid=\"{vp}\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"visinfo.xsd\">\
<Components><Selection>{sel}</Selection><Visibility DefaultVisibility=\"true\"/></Components>\
<PerspectiveCamera>{}{}{}<FieldOfView>{fov}</FieldOfView></PerspectiveCamera></VisualizationInfo>",
            xyz("CameraViewPoint", center - dir * dist),
            xyz("CameraDirection", dir),
            xyz("CameraUpVector", up),
        );
        z.add(&format!("{topic}/markup.bcf"), markup.as_bytes());
        z.add(&format!("{topic}/viewpoint.bcfv"), viewpoint.as_bytes());
    }
    z.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Stored-zip reader for tests: (name, data) per local header.
    fn entries(b: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 0;
        while u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) == 0x0403_4b50 {
            let size = u32::from_le_bytes(b[i + 18..i + 22].try_into().unwrap()) as usize;
            let crc = u32::from_le_bytes(b[i + 14..i + 18].try_into().unwrap());
            let nl = u16::from_le_bytes([b[i + 26], b[i + 27]]) as usize;
            let name = String::from_utf8(b[i + 30..i + 30 + nl].to_vec()).unwrap();
            let data = b[i + 30 + nl..i + 30 + nl + size].to_vec();
            assert_eq!(zip::crc32(&data), crc, "{name}");
            out.push((name, data));
            i += 30 + nl + size;
        }
        out
    }
    fn text<'a>(es: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
        let e = es.iter().find(|(n, _)| n == name).unwrap();
        std::str::from_utf8(&e.1).unwrap()
    }
    #[test]
    fn xlsx_parts_and_cells() {
        let mut p = Project::sample();
        p.elements[0].name = "=cmd()|\"<slab>\" & co".into();
        let b = xlsx(&p);
        assert_eq!(b, xlsx(&p), "deterministic");
        let es = entries(&b);
        let names: Vec<&str> = es.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
                "xl/styles.xml",
                "xl/worksheets/sheet1.xml",
                "xl/worksheets/sheet2.xml"
            ]
        );
        let wb = text(&es, "xl/workbook.xml");
        assert!(wb.contains("name=\"Takeoff\"") && wb.contains("name=\"By phase\""));
        let s1 = text(&es, "xl/worksheets/sheet1.xml");
        assert_eq!(s1.matches("<row ").count(), p.elements.len() + 1);
        // escaped string, and numbers stored as numbers: slab 19.2 m3 at $180 = $3456
        assert!(s1.contains("=cmd()|&quot;&lt;slab&gt;&quot; &amp; co</t>"));
        assert!(s1.contains("<c r=\"D2\" s=\"2\"><v>19.2</v></c>"), "{s1}");
        assert!(s1.contains("<c r=\"F2\" s=\"3\"><v>3456</v></c>"));
        let s2 = text(&es, "xl/worksheets/sheet2.xml");
        assert_eq!(s2.matches("<row ").count(), p.phases.len() + 2);
        assert!(s2.contains("<v>29442.46</v>"), "{s2}");
        assert!(s2.contains("<c r=\"D6\" s=\"1\"><v>57</v></c>"), "{s2}");
    }
    #[test]
    fn ifc_guid_round_trips_through_the_importer() {
        for u in [
            Uuid::nil(),
            Uuid::from_u128(u128::MAX),
            Project::sample().elements[3].id.0,
        ] {
            let g = ifc_guid(u);
            assert_eq!(g.len(), 22);
            assert_eq!(bim_ifc::guid_to_uuid(&g), Some(u));
        }
        // known pair: a buildingSMART-style GlobalId
        let u = bim_ifc::guid_to_uuid("2O2Fr$t4X7Zf8NOew3FLOH").unwrap();
        assert_eq!(ifc_guid(u), "2O2Fr$t4X7Zf8NOew3FLOH");
    }
    #[test]
    fn bcf_topics_per_clash() {
        let p = Project::sample();
        let cl = bim_geom::clashes(&p, 0.001);
        assert_eq!(cl.len(), 4);
        let b = bcf(&p, &cl);
        assert_eq!(b, bcf(&p, &cl), "deterministic");
        let es = entries(&b);
        assert_eq!(es.len(), 2 + 2 * cl.len());
        assert!(text(&es, "bcf.version").contains("VersionId=\"2.1\""));
        let t = clash_topic_guid(&cl[0]);
        assert_eq!(t.get_version_num(), 8);
        let m = text(&es, &format!("{t}/markup.bcf"));
        assert!(m.contains(&format!("<Topic Guid=\"{t}\"")) && m.contains("TopicType=\"Clash\""));
        assert!(m.contains("Column") && m.contains("0.150 m3"), "{m}");
        let v = text(&es, &format!("{t}/viewpoint.bcfv"));
        assert_eq!(v.matches("<Component IfcGuid=").count(), 2);
        assert!(v.contains(&ifc_guid(cl[0].a.0)) && v.contains(&cl[0].b.0.to_string()));
        assert!(v.contains("<PerspectiveCamera><CameraViewPoint>"));
        // topics differ per pair
        let guids: std::collections::HashSet<_> = cl.iter().map(clash_topic_guid).collect();
        assert_eq!(guids.len(), cl.len());
        assert_eq!(entries(&bcf(&p, &[])).len(), 2);
    }
}
