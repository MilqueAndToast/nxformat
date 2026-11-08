//! Dependency-free JSON rendering of an [`Inventory`] for CLI/frontend use.
//! Field names are stable within NX 0.1 tooling; IDs are lowercase hex.

use super::*;
use std::fmt::Write as _;

struct Out(String);

impl Out {
    fn str(&mut self, s: &str) {
        self.0.push('"');
        for c in s.chars() {
            match c {
                '"' => self.0.push_str("\\\""),
                '\\' => self.0.push_str("\\\\"),
                '\n' => self.0.push_str("\\n"),
                '\r' => self.0.push_str("\\r"),
                '\t' => self.0.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(self.0, "\\u{:04x}", c as u32);
                }
                c => self.0.push(c),
            }
        }
        self.0.push('"');
    }
    fn key(&mut self, k: &str) {
        if !matches!(self.0.chars().last(), Some('{') | Some('[')) {
            self.0.push(',');
        }
        self.str(k);
        self.0.push(':');
    }
    fn kv_str(&mut self, k: &str, v: &str) {
        self.key(k);
        self.str(v);
    }
    fn kv_opt_str(&mut self, k: &str, v: Option<&str>) {
        self.key(k);
        match v {
            Some(v) => self.str(v),
            None => self.0.push_str("null"),
        }
    }
    fn kv_raw(&mut self, k: &str, v: impl std::fmt::Display) {
        self.key(k);
        let _ = write!(self.0, "{v}");
    }
    fn open(&mut self, c: char) {
        if !matches!(
            self.0.chars().last(),
            Some('{') | Some('[') | Some(':') | None
        ) {
            self.0.push(',');
        }
        self.0.push(c);
    }
    fn close(&mut self, c: char) {
        self.0.push(c);
    }
}

fn id(v: u64) -> String {
    format!("{v:016x}")
}

fn issue(o: &mut Out, i: &Issue) {
    o.open('{');
    o.kv_str("severity", &format!("{:?}", i.severity).to_lowercase());
    o.kv_str("code", &format!("{:?}", i.code));
    o.kv_str("message", &i.message);
    o.key("source");
    match i.source {
        Some(s) => {
            let _ = write!(o.0, "{s}");
        }
        None => o.0.push_str("null"),
    }
    o.kv_opt_str("title_id", i.title_id.map(id).as_deref());
    o.close('}');
}

pub(super) fn inventory_json(inv: &Inventory) -> String {
    let mut o = Out(String::new());
    o.open('{');
    o.kv_raw("format_version", 1);
    o.key("stats");
    o.open('{');
    o.kv_raw("elapsed_ms", inv.stats.elapsed.as_secs_f64() * 1000.0);
    o.kv_raw("files_examined", inv.stats.files_examined);
    o.kv_raw("files_ignored", inv.stats.files_ignored);
    o.kv_raw("bytes_read", inv.stats.bytes_read);
    o.close('}');
    o.key("sources");
    o.open('[');
    for s in &inv.sources {
        o.open('{');
        o.kv_str("path", &s.path.display().to_string());
        o.kv_raw("size", s.size);
        o.kv_str("format", &format!("{:?}", s.format));
        o.kv_str("status", &format!("{:?}", s.status));
        o.kv_raw("units", s.units.len());
        o.key("metas");
        o.open('[');
        for m in &s.metas {
            let c = &m.meta;
            o.open('{');
            o.kv_str("title_id", &id(c.title_id));
            o.kv_str("application_id", &id(c.application_id));
            o.kv_str("type", &format!("{:?}", c.meta_type));
            o.kv_raw("version", c.version);
            o.kv_str("generation", &format!("{:?}", c.generation));
            o.kv_raw("plannable", m.plannable);
            o.kv_raw("requires_base_romfs", c.requires_base_romfs);
            o.kv_opt_str(
                "required_application_version",
                c.required_application_version
                    .map(|v| v.to_string())
                    .as_deref(),
            );
            o.kv_opt_str("build_id", c.build_id.map(|b| hex(&b)).as_deref());
            o.kv_opt_str(
                "meta_content_id",
                c.meta_content_id.map(|b| hex(&b)).as_deref(),
            );
            o.key("programs");
            o.open('[');
            for program in &c.programs {
                o.open('{');
                o.kv_str("program_id", &id(program.program_id));
                o.kv_str("content_id", &hex(&program.content_id));
                o.kv_raw("id_offset", program.id_offset);
                o.kv_opt_str("build_id", program.build_id.map(|b| hex(&b)).as_deref());
                o.close('}');
            }
            o.close(']');
            o.key("controls");
            o.open('[');
            for control in &c.controls {
                o.open('{');
                o.kv_str("program_id", &id(control.program_id));
                o.kv_raw("id_offset", control.id_offset);
                o.kv_opt_str(
                    "display_version",
                    control.metadata.display_version.as_deref(),
                );
                o.key("titles");
                o.open('[');
                for t in &control.metadata.titles {
                    o.open('{');
                    o.kv_str("language", &t.language);
                    o.kv_opt_str("name", t.name.as_deref());
                    o.kv_opt_str("publisher", t.publisher.as_deref());
                    o.close('}');
                }
                o.close(']');
                o.close('}');
            }
            o.close(']');
            o.kv_str("verification", &format!("{:?}", c.verification));
            o.kv_raw("content_units", m.units.len());
            if let Some(ctl) = &c.control {
                o.kv_opt_str("display_version", ctl.display_version.as_deref());
                o.key("titles");
                o.open('[');
                for t in &ctl.titles {
                    o.open('{');
                    o.kv_str("language", &t.language);
                    o.kv_opt_str("name", t.name.as_deref());
                    o.kv_opt_str("publisher", t.publisher.as_deref());
                    o.close('}');
                }
                o.close(']');
                o.key("icons");
                o.open('[');
                for (lang, size) in &ctl.icons {
                    o.open('{');
                    o.kv_str("language", lang);
                    o.kv_raw("size", size);
                    o.close('}');
                }
                o.close(']');
            }
            o.close('}');
        }
        o.close(']');
        if let Some(t) = &s.tree {
            o.key("tree");
            o.open('{');
            o.kv_opt_str("program_id", t.program_id.map(id).as_deref());
            o.kv_opt_str("npdm_name", t.npdm_name.as_deref());
            o.kv_opt_str(
                "npdm_flags",
                t.npdm_flags.map(|f| format!("{f:#04x}")).as_deref(),
            );
            o.kv_opt_str("main_build_id", t.main_build_id.map(|b| hex(&b)).as_deref());
            o.kv_raw("has_romfs", t.has_romfs);
            o.key("exefs_files");
            o.open('[');
            for name in &t.exefs_files {
                if !o.0.ends_with('[') {
                    o.0.push(',');
                }
                o.str(name);
            }
            o.close(']');
            o.key("nso_build_ids");
            o.open('[');
            for (name, id) in &t.nso_build_ids {
                o.open('{');
                o.kv_str("name", name);
                o.kv_str("build_id", &hex(id));
                o.close('}');
            }
            o.close(']');
            o.close('}');
        }
        o.key("issues");
        o.open('[');
        for i in &s.issues {
            issue(&mut o, i);
        }
        o.close(']');
        o.close('}');
    }
    o.close(']');
    o.key("applications");
    o.open('[');
    for a in &inv.applications {
        o.open('{');
        o.kv_str("platform", &format!("{:?}", a.platform));
        o.kv_str("application_id", &id(a.application_id));
        for (name, refs) in [
            ("bases", &a.bases),
            ("updates", &a.updates),
            ("dlc", &a.dlc),
        ] {
            o.key(name);
            o.open('[');
            for r in refs {
                o.open('{');
                o.kv_str("title_id", &id(r.title_id));
                o.kv_raw("version", r.version);
                o.kv_raw("source", r.source);
                o.close('}');
            }
            o.close(']');
        }
        o.close('}');
    }
    o.close(']');
    o.key("issues");
    o.open('[');
    for i in &inv.issues {
        issue(&mut o, i);
    }
    o.close(']');
    o.close('}');
    o.0
}

#[cfg(test)]
mod tests {
    use super::Out;

    #[test]
    fn strings_are_escaped() {
        let mut o = Out(String::new());
        o.str("a\"b\\c\nd\u{1}é");
        assert_eq!(o.0, r#""a\"b\\c\nd\u0001é""#);
    }
}
