use flate2::write::ZlibEncoder;
use flate2::Compression;
use spoor_core::{Report, SpoorError};
use std::io::Write;

struct PdfWriter {
    objects: Vec<Vec<u8>>,
}

impl PdfWriter {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn add_object(&mut self, content: Vec<u8>) -> usize {
        self.objects.push(content);
        self.objects.len()
    }

    fn build(self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");

        let mut offsets = Vec::new();
        for (i, obj) in self.objects.iter().enumerate() {
            let obj_num = i + 1;
            offsets.push(buf.len());
            buf.extend_from_slice(format!("{obj_num} 0 obj\n").as_bytes());
            buf.extend_from_slice(obj);
            buf.extend_from_slice(b"\nendobj\n");
        }

        let xref_offset = buf.len();
        buf.extend_from_slice(format!("xref\n0 {}\n", self.objects.len() + 1).as_bytes());
        buf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            buf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }

        buf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                self.objects.len() + 1,
                xref_offset
            )
            .as_bytes(),
        );

        buf
    }
}

fn sanitize_pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' | '\n' | '\t' => out.push(' '),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            // WinAnsi / ISO-8859-1 accented characters (0xA0..=0xFF) encoded via exact octal escapes
            c if (c as u32) >= 0x00A0 && (c as u32) <= 0x00FF => {
                let byte = c as u8;
                out.push_str(&format!("\\{:03o}", byte));
            }
            // Typographical punctuation in Windows-1252
            '\u{2018}' => out.push_str("\\221"),
            '\u{2019}' => out.push_str("\\222"),
            '\u{201C}' => out.push_str("\\223"),
            '\u{201D}' => out.push_str("\\224"),
            '\u{2022}' => out.push_str("\\225"),
            '\u{2013}' => out.push_str("\\226"),
            '\u{2014}' => out.push_str("\\227"),
            '\u{2026}' => out.push_str("\\205"),
            '\u{20AC}' => out.push_str("\\200"),
            '\u{0152}' => out.push_str("\\214"),
            '\u{0153}' => out.push_str("\\234"),
            '\u{0160}' => out.push_str("\\212"),
            '\u{0161}' => out.push_str("\\232"),
            '\u{0178}' => out.push_str("\\237"),
            '\u{017D}' => out.push_str("\\216"),
            '\u{017E}' => out.push_str("\\236"),
            _ => out.push(' '),
        }
    }
    out
}

fn wrap_text(s: &str, max_chars: usize) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in s.split_whitespace() {
        if current.is_empty() {
            if word.len() > max_chars {
                let mut remaining = word;
                while remaining.len() > max_chars {
                    let (head, tail) = remaining.split_at(max_chars);
                    lines.push(head.to_string());
                    remaining = tail;
                }
                current.push_str(remaining);
            } else {
                current.push_str(word);
            }
        } else if current.len() + 1 + word.len() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = String::new();
            if word.len() > max_chars {
                let mut remaining = word;
                while remaining.len() > max_chars {
                    let (head, tail) = remaining.split_at(max_chars);
                    lines.push(head.to_string());
                    remaining = tail;
                }
                current.push_str(remaining);
            } else {
                current.push_str(word);
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

struct PdfLayoutEngine {
    pages: Vec<Vec<u8>>,
    current: Vec<u8>,
    y: f32,
    target_display: String,
    mode_str: String,
    content_width: f32,
    left_margin: f32,
    bottom_margin: f32,
}

impl PdfLayoutEngine {
    fn new(target_display: String, mode_str: String) -> Self {
        let left_margin = 40.0;
        let content_width = 515.0;
        let bottom_margin = 65.0;
        let mut engine = Self {
            pages: Vec::new(),
            current: Vec::new(),
            y: 790.0,
            target_display,
            mode_str,
            content_width,
            left_margin,
            bottom_margin,
        };
        engine.init_first_page();
        engine
    }

    fn init_first_page(&mut self) {
        let p = &mut self.current;
        p.extend_from_slice(b"0.08 0.10 0.14 rg\n");
        p.extend_from_slice(b"40 760 515 48 re f\n");

        p.extend_from_slice(b"BT /F2 18 Tf 1 1 1 rg 55 778 Td (SPOOR) Tj ET\n");
        p.extend_from_slice(b"BT /F1 10 Tf 0.6 0.7 0.8 rg 135 778 Td (by Voldigoade) Tj ET\n");

        let mode_label = format!("MODE: {}", self.mode_str);
        p.extend_from_slice(
            format!(
                "BT /F2 10 Tf 0.35 0.65 1 rg 460 778 Td ({}) Tj ET\n",
                sanitize_pdf_text(&mode_label)
            )
            .as_bytes(),
        );

        self.y = 740.0;
    }

    fn new_continuation_page(&mut self) {
        let old_page = std::mem::take(&mut self.current);
        self.pages.push(old_page);

        let p = &mut self.current;
        p.extend_from_slice(b"0.08 0.10 0.14 rg\n");
        p.extend_from_slice(b"40 780 515 26 re f\n");

        p.extend_from_slice(
            b"BT /F2 10 Tf 1 1 1 rg 50 788 Td (SPOOR - REPORT CONTINUATION) Tj ET\n",
        );
        let sub = format!("Target: {} | {}", self.target_display, self.mode_str);
        p.extend_from_slice(
            format!(
                "BT /F1 8 Tf 0.7 0.8 0.9 rg 340 788 Td ({}) Tj ET\n",
                sanitize_pdf_text(&sub)
            )
            .as_bytes(),
        );

        self.y = 755.0;
    }

    fn ensure_space(&mut self, required_height: f32) {
        if self.y - required_height < self.bottom_margin {
            self.new_continuation_page();
        }
    }

    fn add_section_header(&mut self, title: &str) {
        self.ensure_space(32.0);
        let p = &mut self.current;
        let y_text = self.y - 12.0;
        let y_line = self.y - 16.0;

        p.extend_from_slice(
            format!(
                "BT /F2 11 Tf 0.15 0.20 0.30 rg {} {} Td ({}) Tj ET\n",
                self.left_margin,
                y_text,
                sanitize_pdf_text(title)
            )
            .as_bytes(),
        );
        p.extend_from_slice(
            format!(
                "0.20 0.40 0.80 RG {} {} {} 0.5 re S\n",
                self.left_margin, y_line, self.content_width
            )
            .as_bytes(),
        );

        self.y -= 26.0;
    }

    fn add_table_header(&mut self, cols: &[(&str, f32)]) {
        self.ensure_space(18.0);
        let p = &mut self.current;
        let h_y = self.y - 14.0;

        p.extend_from_slice(
            format!(
                "0.94 0.95 0.96 rg {} {} {} 14 re f\n",
                self.left_margin, h_y, self.content_width
            )
            .as_bytes(),
        );

        let mut current_x = self.left_margin + 5.0;
        for (name, width) in cols {
            p.extend_from_slice(
                format!(
                    "BT /F2 8 Tf 0.2 0.25 0.3 rg {} {} Td ({}) Tj ET\n",
                    current_x,
                    h_y + 3.5,
                    sanitize_pdf_text(name)
                )
                .as_bytes(),
            );
            current_x += width;
        }

        self.y -= 16.0;
    }

    fn add_table_row(&mut self, cols: &[(&str, f32, bool)]) {
        self.ensure_space(14.0);
        let p = &mut self.current;
        let row_y = self.y - 11.0;

        let mut current_x = self.left_margin + 5.0;
        for (val, width, is_bold) in cols {
            let font = if *is_bold { "/F2" } else { "/F1" };
            let max_chars = (*width / 5.0) as usize;
            let fitted_val = if val.chars().count() > max_chars && max_chars > 3 {
                let truncated: String = val.chars().take(max_chars - 1).collect();
                format!("{truncated}...")
            } else {
                val.to_string()
            };

            p.extend_from_slice(
                format!(
                    "BT {} 8 Tf 0.15 0.15 0.15 rg {} {} Td ({}) Tj ET\n",
                    font,
                    current_x,
                    row_y,
                    sanitize_pdf_text(&fitted_val)
                )
                .as_bytes(),
            );
            current_x += width;
        }

        self.y -= 13.0;
    }

    fn add_key_value(&mut self, key: &str, val: &str) {
        let max_val_chars = 65;
        let val_lines = wrap_text(val, max_val_chars);
        let first_val = val_lines.first().map(|s| s.as_str()).unwrap_or("");

        self.ensure_space(14.0);
        let p = &mut self.current;
        let row_y = self.y - 11.0;

        p.extend_from_slice(
            format!(
                "BT /F2 9 Tf 0.3 0.35 0.4 rg {} {} Td ({}:) Tj ET\n\
                 BT /F1 9 Tf 0.1 0.1 0.1 rg {} {} Td ({}) Tj ET\n",
                self.left_margin + 10.0,
                row_y,
                sanitize_pdf_text(key),
                self.left_margin + 170.0,
                row_y,
                sanitize_pdf_text(first_val)
            )
            .as_bytes(),
        );

        self.y -= 14.0;

        for line in val_lines.iter().skip(1) {
            self.ensure_space(12.0);
            let p = &mut self.current;
            let sub_y = self.y - 10.0;
            p.extend_from_slice(
                format!(
                    "BT /F1 9 Tf 0.1 0.1 0.1 rg {} {} Td ({}) Tj ET\n",
                    self.left_margin + 170.0,
                    sub_y,
                    sanitize_pdf_text(line)
                )
                .as_bytes(),
            );
            self.y -= 12.0;
        }
    }

    fn add_card_target(
        &mut self,
        target_name: &str,
        target_type: &str,
        generated: &str,
        canonical_url: &str,
    ) {
        self.ensure_space(65.0);
        let p = &mut self.current;
        let box_y = self.y - 60.0;

        p.extend_from_slice(
            format!(
                "0.96 0.97 0.98 rg {} {} {} 60 re f\n\
                 0.85 0.88 0.92 RG {} {} {} 60 re S\n",
                self.left_margin,
                box_y,
                self.content_width,
                self.left_margin,
                box_y,
                self.content_width
            )
            .as_bytes(),
        );

        p.extend_from_slice(
            format!(
                "BT /F2 13 Tf 0.10 0.15 0.20 rg {} {} Td (TARGET: {}) Tj ET\n\
                 BT /F1 9 Tf 0.40 0.45 0.50 rg {} {} Td (Generated: {}   |   Target Type: {}) Tj ET\n\
                 BT /F1 9 Tf 0.40 0.45 0.50 rg {} {} Td (URL: {}) Tj ET\n",
                self.left_margin + 15.0,
                box_y + 40.0,
                sanitize_pdf_text(target_name),
                self.left_margin + 15.0,
                box_y + 24.0,
                sanitize_pdf_text(generated),
                sanitize_pdf_text(target_type),
                self.left_margin + 15.0,
                box_y + 10.0,
                sanitize_pdf_text(canonical_url)
            )
            .as_bytes(),
        );

        self.y -= 70.0;
    }

    fn finalize_and_get_pages(mut self) -> Vec<Vec<u8>> {
        let last_page = std::mem::take(&mut self.current);
        self.pages.push(last_page);

        let total_pages = self.pages.len();
        for (idx, page) in self.pages.iter_mut().enumerate() {
            let page_num = idx + 1;
            page.extend_from_slice(b"0.80 0.80 0.80 RG 40 50 515 0.5 re S\n");
            page.extend_from_slice(
                b"BT /F1 8 Tf 0.50 0.50 0.50 rg 40 38 Td (Spoor v0.2.0 | by Voldigoade) Tj ET\n",
            );
            let page_str = format!("Page {} of {}", page_num, total_pages);
            page.extend_from_slice(
                format!(
                    "BT /F1 8 Tf 0.50 0.50 0.50 rg 505 38 Td ({}) Tj ET\n",
                    sanitize_pdf_text(&page_str)
                )
                .as_bytes(),
            );

            if page_num == 1 {
                page.extend_from_slice(b"BT /F1 7.5 Tf 0.50 0.50 0.50 rg 40 62 Td (* Sourced strictly from public Git commit headers and GitHub public API endpoints.) Tj ET\n");
            }
        }

        self.pages
    }
}

fn compress_stream(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_else(|_| data.to_vec())
}

pub fn export_pdf(report: &Report) -> Result<Vec<u8>, SpoorError> {
    let target_name = report.target.display_name();
    let mode_str = report.mode.to_string().to_uppercase();
    let mut engine = PdfLayoutEngine::new(target_name.clone(), mode_str);

    let generated_str = report
        .generated_at
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string();
    engine.add_card_target(
        &target_name,
        &report.summary.target_type,
        &generated_str,
        &report.target.canonical_url(),
    );

    engine.add_section_header("1. SUMMARY OF FINDINGS");
    engine.add_key_value("Display Name", &report.summary.display_name);
    engine.add_key_value(
        "Repositories Cataloged",
        &report.summary.total_public_repositories.to_string(),
    );
    engine.add_key_value(
        "Total Stars Accumulated",
        &report.summary.total_stars_accumulated.to_string(),
    );
    engine.add_key_value(
        "Total Forks Accumulated",
        &report.summary.total_forks_accumulated.to_string(),
    );
    engine.add_key_value(
        "Distinct Email Identities",
        &report.summary.distinct_emails_observed.to_string(),
    );
    engine.add_key_value(
        "Public Keys Found",
        &report.summary.public_keys_found.to_string(),
    );
    engine.add_key_value(
        "Fork Ratio",
        &format!("{:.1}%", report.summary.fork_ratio * 100.0),
    );
    engine.add_key_value(
        "Archived Ratio",
        &format!("{:.1}%", report.summary.archived_ratio * 100.0),
    );

    engine.add_section_header("2. COVERAGE & AUDIT RECORD");
    engine.add_key_value(
        "Commits Scanned",
        &report.coverage.commits_scanned.to_string(),
    );
    engine.add_key_value(
        "Repositories Scanned",
        &format!(
            "{}/{}",
            report.coverage.repositories_scanned, report.coverage.repositories_total
        ),
    );
    engine.add_key_value(
        "Candidate Repositories Found",
        &report.coverage.candidate_repositories_found.to_string(),
    );
    engine.add_key_value(
        "Global Commits Found",
        &report.coverage.global_commits_found.to_string(),
    );
    engine.add_key_value(
        "Git History Inspected",
        if report.coverage.git_history_inspected {
            "Yes (Bare Clone Metadata Scan)"
        } else {
            "No (API-only)"
        },
    );
    engine.add_key_value(
        "Coverage State",
        if report.coverage.is_partial {
            "Partial (Budget limited)"
        } else {
            "Complete"
        },
    );
    if let Some(ref notes) = report.coverage.notes {
        engine.add_key_value("Coverage Notes", notes);
    }

    engine.add_section_header("3. PUBLIC IDENTITIES OBSERVED IN METADATA");
    let id_cols = [
        ("EMAIL", 185.0),
        ("COMMITS", 60.0),
        ("CLASSIFICATION", 190.0),
        ("VERIFIED", 70.0),
    ];
    engine.add_table_header(&id_cols);

    if report.identities.is_empty() {
        engine.ensure_space(14.0);
        let p = &mut engine.current;
        let row_y = engine.y - 11.0;
        p.extend_from_slice(
            format!(
                "BT /F1 8 Tf 0.5 0.5 0.5 rg 45 {} Td (No public commit or profile emails identified) Tj ET\n",
                row_y
            )
            .as_bytes(),
        );
        engine.y -= 14.0;
    } else {
        for id in &report.identities {
            let commits_str = id.commits_observed.to_string();
            let verified_str = if id.is_verified_signer { "Yes" } else { "No" };
            let row = [
                (id.email.as_str(), 185.0, true),
                (commits_str.as_str(), 60.0, false),
                (id.email_type.as_str(), 190.0, false),
                (verified_str, 70.0, false),
            ];
            engine.add_table_row(&row);
        }
    }

    engine.add_section_header("4. PUBLIC REPOSITORIES");
    let repo_cols = [
        ("REPOSITORY", 200.0),
        ("LANGUAGE", 110.0),
        ("STARS", 65.0),
        ("FORKS", 65.0),
        ("STATE", 65.0),
    ];
    engine.add_table_header(&repo_cols);

    if report.repositories.is_empty() {
        engine.ensure_space(14.0);
        let p = &mut engine.current;
        let row_y = engine.y - 11.0;
        p.extend_from_slice(
            format!(
                "BT /F1 8 Tf 0.5 0.5 0.5 rg 45 {} Td (No public repositories cataloged) Tj ET\n",
                row_y
            )
            .as_bytes(),
        );
        engine.y -= 14.0;
    } else {
        for r in &report.repositories {
            let lang = r.primary_language.as_deref().unwrap_or("-");
            let stars_str = r.stars.to_string();
            let forks_str = r.forks.to_string();
            let state = if r.is_archived {
                "Archived"
            } else if r.is_fork {
                "Fork"
            } else {
                "Active"
            };

            let row = [
                (r.name.as_str(), 200.0, true),
                (lang, 110.0, false),
                (stars_str.as_str(), 65.0, false),
                (forks_str.as_str(), 65.0, false),
                (state, 65.0, false),
            ];
            engine.add_table_row(&row);
        }
    }

    if !report.keys.is_empty() {
        engine.add_section_header("5. PUBLIC KEYS & AUTHENTICATION MATERIAL");
        for k in &report.keys {
            let key_label = format!("[{}]", k.key_type);
            let key_val = format!(
                "ID: {} | Fingerprint: {}",
                k.key_id.as_deref().unwrap_or("-"),
                k.fingerprint.as_deref().unwrap_or("-")
            );
            engine.add_key_value(&key_label, &key_val);
        }
    }

    if !report.inferences.is_empty() {
        engine.add_section_header("6. INFERENCES");
        for inf in &report.inferences {
            engine.add_key_value(&inf.topic, &inf.inference);
        }
    }

    engine.add_section_header("7. PROVIDER STATUS & HEALTH");
    let provider_cols = [
        ("PROVIDER", 190.0),
        ("HEALTH", 90.0),
        ("ITEMS", 65.0),
        ("DETAIL", 160.0),
    ];
    engine.add_table_header(&provider_cols);
    for (p_name, st) in &report.provider_status {
        let status_str = st.status.to_string();
        let items_str = st
            .items_retrieved
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".into());
        let msg = st.message.as_deref().unwrap_or("-");
        let row = [
            (p_name.as_str(), 190.0, true),
            (status_str.as_str(), 90.0, false),
            (items_str.as_str(), 65.0, false),
            (msg, 160.0, false),
        ];
        engine.add_table_row(&row);
    }

    if !report.limitations.is_empty() {
        engine.add_section_header("8. INHERENT LIMITATIONS");
        for lim in &report.limitations {
            let bullet = format!("* {lim}");
            let lines = wrap_text(&bullet, 110);
            for line in lines {
                engine.ensure_space(13.0);
                let p = &mut engine.current;
                let row_y = engine.y - 10.0;
                p.extend_from_slice(
                    format!(
                        "BT /F1 7.5 Tf 0.35 0.40 0.45 rg 45 {} Td ({}) Tj ET\n",
                        row_y,
                        sanitize_pdf_text(&line)
                    )
                    .as_bytes(),
                );
                engine.y -= 12.0;
            }
        }
    }

    let pages_raw = engine.finalize_and_get_pages();
    let page_count = pages_raw.len();

    let mut pdf = PdfWriter::new();
    pdf.add_object(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());

    let mut kids = Vec::new();
    let first_page_obj = 6;
    for i in 0..page_count {
        let page_obj_id = first_page_obj + (i * 2);
        kids.push(format!("{page_obj_id} 0 R"));
    }

    let pages_dict = format!(
        "<< /Type /Pages /Kids [ {} ] /Count {} /MediaBox [ 0 0 595.28 841.89 ] >>",
        kids.join(" "),
        page_count
    );
    pdf.add_object(pages_dict.into_bytes());

    pdf.add_object(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    );
    pdf.add_object(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    pdf.add_object(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec(),
    );

    for (i, page_data) in pages_raw.into_iter().enumerate() {
        let content_obj_id = first_page_obj + (i * 2) + 1;
        let page_dict = format!(
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> /Contents {content_obj_id} 0 R >>"
        );
        pdf.add_object(page_dict.into_bytes());

        let compressed = compress_stream(&page_data);
        let mut stream_obj = format!(
            "<< /Filter /FlateDecode /Length {} >>\nstream\n",
            compressed.len()
        )
        .into_bytes();
        stream_obj.extend_from_slice(&compressed);
        stream_obj.extend_from_slice(b"\nendstream");
        pdf.add_object(stream_obj);
    }

    Ok(pdf.build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoor_core::{AnalysisMode, Target};

    #[test]
    fn generates_valid_pdf_structure() {
        let report = Report::new(
            Target::User {
                username: "torvalds".into(),
            },
            AnalysisMode::Fast,
        );
        let bytes = export_pdf(&report).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        let pdf_str = String::from_utf8_lossy(&bytes);
        assert!(pdf_str.contains("/Type /Catalog"));
        assert!(pdf_str.contains("/Type /Pages"));
        assert!(pdf_str.contains("xref"));
    }

    #[test]
    fn generates_multi_page_pdf_when_data_exceeds_single_page() {
        let mut report = Report::new(
            Target::User {
                username: "torvalds".into(),
            },
            AnalysisMode::Deep,
        );
        for i in 0..50 {
            report.repositories.push(spoor_core::RepoReport {
                name: format!("kernel-module-{i}"),
                full_name: format!("torvalds/kernel-module-{i}"),
                description: Some("Linux kernel subsystem component".into()),
                is_fork: false,
                is_archived: false,
                default_branch: "master".into(),
                stars: 1000 + i as u64,
                forks: 200 + i as u64,
                open_issues: 5,
                size_kb: 45000,
                primary_language: Some("C".into()),
                languages: None,
                created_at: None,
                updated_at: None,
                pushed_at: None,
                topics: vec!["kernel".into(), "os".into()],
                license_spdx: Some("GPL-2.0-only".into()),
                hygiene: None,
                html_url: format!("https://github.com/torvalds/kernel-module-{i}"),
            });
        }
        let bytes = export_pdf(&report).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        let pdf_str = String::from_utf8_lossy(&bytes);
        assert!(pdf_str.contains("/Count "));
        assert!(!pdf_str.contains("/Count 1 "));
    }

    #[test]
    fn test_pdf_preserves_accents_and_winansi_encoding() {
        let mut report = Report::new(
            Target::User {
                username: "voldigoade".into(),
            },
            AnalysisMode::Fast,
        );
        report.summary.display_name = "José-François Müller".into();
        report.identities.push(spoor_core::IdentityRecord {
            name: Some("Théodore Günther".into()),
            email: "contact@école-polytechnique.fr".into(),
            email_type: spoor_core::EmailType::DomainEmail,
            first_seen: None,
            last_seen: None,
            commits_observed: 10,
            repositories: vec!["repo".into()],
            sources: vec!["Git".into()],
            is_verified_signer: true,
        });

        let bytes = export_pdf(&report).unwrap();
        let pdf_str = String::from_utf8_lossy(&bytes);
        assert!(pdf_str.contains("/Encoding /WinAnsiEncoding"));

        let sanitized = sanitize_pdf_text("José-François Müller");
        assert_eq!(sanitized, "Jos\\351-Fran\\347ois M\\374ller");
        assert!(!sanitized.contains("Jose-Francois Muller"));

        let sanitized_email = sanitize_pdf_text("contact@école-polytechnique.fr");
        assert!(sanitized_email.contains("\\351cole"));
    }

    #[test]
    fn test_wrap_text_splits_long_lines() {
        let text = "This is a very long sentence designed to verify that the PDF export engine correctly wraps text onto multiple lines instead of clipping horizontally or overflowing bounding boxes.";
        let lines = wrap_text(text, 40);
        assert!(lines.len() >= 4);
        for line in &lines {
            assert!(line.len() <= 40);
        }
    }
}
