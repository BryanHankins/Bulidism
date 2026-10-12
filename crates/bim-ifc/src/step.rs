//! Minimal ISO 10303-21 (STEP physical file) reader: enough to walk an IFC DATA section.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    /// `*`: value derived elsewhere
    Derived,
    Int(i64),
    Real(f64),
    Str(String),
    Enum(String),
    Ref(u64),
    List(Vec<Value>),
    /// Typed value such as `IFCLABEL('x')` or `IFCBOOLEAN(.T.)`
    Typed(String, Vec<Value>),
}

impl Value {
    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Real(r) => Some(*r),
            Value::Typed(_, v) if v.len() == 1 => v[0].num(),
            _ => None,
        }
    }
    pub fn id(&self) -> Option<u64> {
        match self {
            Value::Ref(r) => Some(*r),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            Value::Typed(_, v) if v.len() == 1 => v[0].str(),
            _ => None,
        }
    }
    pub fn enumv(&self) -> Option<&str> {
        match self {
            Value::Enum(s) => Some(s),
            Value::Typed(_, v) if v.len() == 1 => v[0].enumv(),
            _ => None,
        }
    }
    pub fn list(&self) -> &[Value] {
        match self {
            Value::List(v) => v,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// Upper-case type name, e.g. `IFCWALL`.
    pub ty: String,
    pub args: Vec<Value>,
}

impl Entity {
    pub fn arg(&self, i: usize) -> &Value {
        self.args.get(i).unwrap_or(&Value::Null)
    }
}

#[derive(Debug, Default)]
pub struct StepFile {
    /// First schema in FILE_SCHEMA, e.g. `IFC2X3` or `IFC4`.
    pub schema: String,
    pub entities: HashMap<u64, Entity>,
}

#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("not an IFC/STEP file (no ISO-10303-21 header)")]
    NotStep,
    #[error("no DATA section")]
    NoData,
    #[error("syntax error near byte {0}: {1}")]
    Syntax(usize, &'static str),
}

struct Cur<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    fn ws(&mut self) {
        loop {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.b.get(self.i..).is_some_and(|r| r.starts_with(b"/*")) {
                match find(&self.b[self.i + 2..], b"*/") {
                    Some(p) => self.i += p + 4,
                    None => self.i = self.b.len(),
                }
            } else {
                return;
            }
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn eat(&mut self, c: u8, what: &'static str) -> Result<(), StepError> {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(StepError::Syntax(self.i, what))
        }
    }
    fn keyword(&mut self) -> String {
        let s = self.i;
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_')
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[s..self.i]).to_ascii_uppercase()
    }
    fn digits(&mut self) -> u64 {
        let mut n = 0u64;
        while let Some(c @ b'0'..=b'9') = self.peek() {
            n = n.saturating_mul(10).saturating_add((c - b'0') as u64);
            self.i += 1;
        }
        n
    }
    /// `( v, v, ... )`
    fn list(&mut self) -> Result<Vec<Value>, StepError> {
        self.eat(b'(', "expected (")?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == Some(b')') {
            self.i += 1;
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b')') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err(StepError::Syntax(self.i, "expected , or )")),
            }
        }
    }
    fn value(&mut self) -> Result<Value, StepError> {
        self.ws();
        let c = self
            .peek()
            .ok_or(StepError::Syntax(self.i, "unexpected end"))?;
        Ok(match c {
            b'$' => {
                self.i += 1;
                Value::Null
            }
            b'*' => {
                self.i += 1;
                Value::Derived
            }
            b'#' => {
                self.i += 1;
                Value::Ref(self.digits())
            }
            b'(' => Value::List(self.list()?),
            b'.' => {
                self.i += 1;
                let s = self.i;
                while self.i < self.b.len() && self.b[self.i] != b'.' {
                    self.i += 1;
                }
                let e = String::from_utf8_lossy(&self.b[s..self.i]).into_owned();
                self.i = (self.i + 1).min(self.b.len()); // past the closing '.', never past the end
                Value::Enum(e)
            }
            b'\'' => Value::Str(self.string()?),
            b'"' => {
                // binary: keep the hex text
                self.i += 1;
                let s = self.i;
                while self.i < self.b.len() && self.b[self.i] != b'"' {
                    self.i += 1;
                }
                let v = String::from_utf8_lossy(&self.b[s..self.i]).into_owned();
                self.i = (self.i + 1).min(self.b.len());
                Value::Str(v)
            }
            b'0'..=b'9' | b'-' | b'+' => {
                let s = self.i;
                self.i += 1;
                while let Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'-' | b'+') = self.peek() {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[s..self.i]).unwrap_or("0");
                if t.contains(['.', 'e', 'E']) {
                    Value::Real(t.parse().map_err(|_| StepError::Syntax(s, "bad number"))?)
                } else {
                    Value::Int(t.parse().map_err(|_| StepError::Syntax(s, "bad number"))?)
                }
            }
            b'A'..=b'Z' | b'a'..=b'z' => {
                let k = self.keyword();
                Value::Typed(k, self.list()?)
            }
            _ => return Err(StepError::Syntax(self.i, "unexpected character")),
        })
    }
    /// `'...'` with `''` for a quote and ISO 10303-21 `\X2\ \X\ \S\ \\` escapes.
    fn string(&mut self) -> Result<String, StepError> {
        self.i += 1;
        let mut raw = Vec::new();
        loop {
            match self.peek() {
                None => return Err(StepError::Syntax(self.i, "unterminated string")),
                Some(b'\'') if self.b.get(self.i + 1) == Some(&b'\'') => {
                    raw.push(b'\'');
                    self.i += 2;
                }
                Some(b'\'') => {
                    self.i += 1;
                    break;
                }
                Some(c) => {
                    raw.push(c);
                    self.i += 1;
                }
            }
        }
        Ok(decode(&String::from_utf8_lossy(&raw)))
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// Decode STEP string escapes.
pub fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('\\') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        if let Some(r) = rest.strip_prefix("\\X2\\") {
            let end = r.find("\\X0\\").unwrap_or(r.len());
            let units: Vec<u16> = r.as_bytes()[..end]
                .chunks(4)
                .filter_map(|c| u16::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
                .collect();
            out.push_str(&String::from_utf16_lossy(&units));
            rest = r.get(end + 4..).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix("\\X\\") {
            match r.get(..2).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b as char);
                    rest = &r[2..];
                }
                None => {
                    out.push('\\');
                    rest = &rest[1..];
                }
            }
        } else if let Some(r) = rest.strip_prefix("\\S\\") {
            match r.chars().next() {
                Some(c) => {
                    out.push(char::from_u32(c as u32 + 128).unwrap_or(c));
                    rest = &r[c.len_utf8()..];
                }
                None => rest = r,
            }
        } else if let Some(r) = rest.strip_prefix("\\\\") {
            out.push('\\');
            rest = r;
        } else {
            out.push('\\');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

pub fn parse(text: &str) -> Result<StepFile, StepError> {
    let b = text.as_bytes();
    if !text.trim_start().starts_with("ISO-10303-21") {
        return Err(StepError::NotStep);
    }
    let mut f = StepFile::default();
    if let Some(p) = text.find("FILE_SCHEMA") {
        let rest = &text[p..];
        if let (Some(a), Some(z)) = (rest.find('\''), rest[1..].find("')")) {
            if a < z + 1 {
                f.schema = rest[a + 1..z + 1].to_ascii_uppercase();
            }
        }
    }
    let data = find(b, b"DATA;").ok_or(StepError::NoData)?;
    let mut c = Cur { b, i: data + 5 };
    loop {
        c.ws();
        match c.peek() {
            None => break,
            Some(b'#') => {
                c.i += 1;
                let id = c.digits();
                c.eat(b'=', "expected =")?;
                c.ws();
                if c.peek() == Some(b'(') {
                    // complex instance (several types): keep the first part
                    c.i += 1;
                    c.ws();
                    let ty = c.keyword();
                    let args = c.list()?;
                    let mut depth = 1;
                    while depth > 0 && c.i < b.len() {
                        match b[c.i] {
                            b'(' => depth += 1,
                            b')' => depth -= 1,
                            b'\'' => {
                                c.string()?;
                                continue;
                            }
                            _ => {}
                        }
                        c.i += 1;
                    }
                    f.entities.insert(id, Entity { ty, args });
                } else {
                    let ty = c.keyword();
                    let args = c.list()?;
                    f.entities.insert(id, Entity { ty, args });
                }
                c.eat(b';', "expected ;")?;
            }
            Some(_) => {
                // ENDSEC; or anything after the data section
                if b[c.i..].starts_with(b"ENDSEC") {
                    break;
                }
                return Err(StepError::Syntax(c.i, "expected #id"));
            }
        }
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_values_and_escapes() {
        let f = parse(
            "ISO-10303-21;\nHEADER;FILE_SCHEMA(('IFC4'));ENDSEC;\nDATA;\n\
             #1=IFCWALL('2O2Fr$t4X7Zf8NOew3FLOH',#2,'It''s a w\\X2\\00E9\\X0\\ll',$,*,(1,-2.5,3.E2),.T.,IFCLABEL('x'));\n\
             /* comment */ #2 = IFCX(\n (), ((#1)) );\nENDSEC;\nEND-ISO-10303-21;",
        )
        .unwrap();
        assert_eq!(f.schema, "IFC4");
        let w = &f.entities[&1];
        assert_eq!(w.ty, "IFCWALL");
        assert_eq!(w.arg(2).str(), Some("It's a wéll"));
        assert_eq!(w.arg(3), &Value::Null);
        assert_eq!(w.arg(4), &Value::Derived);
        let l: Vec<f64> = w.arg(5).list().iter().filter_map(|v| v.num()).collect();
        assert_eq!(l, vec![1.0, -2.5, 300.0]);
        assert_eq!(w.arg(6).enumv(), Some("T"));
        assert_eq!(w.arg(7).str(), Some("x"));
        assert_eq!(f.entities[&2].arg(1).list()[0].list()[0].id(), Some(1));
    }
    #[test]
    fn truncated_files_error_instead_of_panicking() {
        let full = include_str!("../tests/data/fixture.ifc");
        for cut in (0..full.len()).filter(|c| full.is_char_boundary(*c)) {
            let _ = parse(&full[..cut]);
            let _ = crate::import(&full[..cut], "t");
        }
        for bad in [
            "ISO-10303-21;\nDATA;\n#1=X(.T",
            "ISO-10303-21;\nDATA;\n#1=X(\"AB",
            "ISO-10303-21;\nDATA;\n#1=(X(1)",
            "ISO-10303-21;\nDATA;\n/*",
        ] {
            let _ = parse(bad);
        }
    }
    #[test]
    fn rejects_non_step() {
        assert!(matches!(parse("{}"), Err(StepError::NotStep)));
        assert!(parse("ISO-10303-21;\nDATA;\n#1=IFCX(;\nENDSEC;").is_err());
    }
}
