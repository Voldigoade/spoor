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

fn escape_pdf_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' => {}
            '\n' => out.push(' '),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

pub fn export_pdf(report: &Report) -> Result<Vec<u8>, SpoorError> {
    let mut pdf = PdfWriter::new();

    // We will generate a structured multi-page PDF document
    // Let's divide content into pages if necessary (Page 1: Summary, Profile, Identities; Page 2: Repos, Keys, Coverage, Limitations)
    let page_count = 2;

    // 1: Catalog
    // 2: Pages tree
    // 3: Font F1 (Helvetica)
    // 4: Font F2 (Helvetica-Bold)
    // 5: Font F3 (Courier)
    // 6: Page 1
    // 7: Contents for Page 1
    // 8: Page 2
    // 9: Contents for Page 2

    // Page 1 Content
    let mut p1 = Vec::new();
    // Top banner
    p1.extend_from_slice(b"0.08 0.1 0.14 rg\n"); // dark header background
    p1.extend_from_slice(b"40 760 515 50 re f\n");

    // Header Title
    p1.extend_from_slice(b"BT /F2 18 Tf 1 1 1 rg 55 780 Td (SPOOR) Tj ET\n");
    p1.extend_from_slice(b"BT /F1 10 Tf 0.6 0.7 0.8 rg 135 780 Td (by Voldigoade) Tj ET\n");
    let mode_str = format!("MODE: {}", report.mode.to_string().to_uppercase());
    p1.extend_from_slice(
        format!(
            "BT /F2 10 Tf 0.35 0.65 1 rg 460 780 Td ({}) Tj ET\n",
            escape_pdf_text(&mode_str)
        )
        .as_bytes(),
    );

    // Target Info Box
    p1.extend_from_slice(b"0.96 0.97 0.98 rg 40 685 515 65 re f\n");
    p1.extend_from_slice(b"0.85 0.88 0.92 RG 40 685 515 65 re S\n");

    p1.extend_from_slice(
        format!(
            "BT /F2 13 Tf 0.1 0.15 0.2 rg 55 725 Td (TARGET: {}) Tj ET\n",
            escape_pdf_text(&report.target.display_name())
        )
        .as_bytes(),
    );
    p1.extend_from_slice(
        format!(
            "BT /F1 9 Tf 0.4 0.45 0.5 rg 55 708 Td (Generated: {}   |   Target Type: {}) Tj ET\n",
            report.generated_at.format("%Y-%m-%d %H:%M:%S UTC"),
            escape_pdf_text(&report.summary.target_type)
        )
        .as_bytes(),
    );
    p1.extend_from_slice(
        format!(
            "BT /F1 9 Tf 0.4 0.45 0.5 rg 55 693 Td (URL: {}) Tj ET\n",
            escape_pdf_text(&report.target.canonical_url())
        )
        .as_bytes(),
    );

    // Section 1: Summary Facts Grid
    p1.extend_from_slice(
        b"BT /F2 12 Tf 0.15 0.2 0.3 rg 40 660 Td (1. SUMMARY OF FINDINGS) Tj ET\n",
    );
    p1.extend_from_slice(b"0.2 0.4 0.8 RG 40 655 515 0.5 re S\n");

    let mut y = 638;
    let summary_items = [
        ("Display Name", report.summary.display_name.clone()),
        (
            "Repositories Cataloged",
            format!("{}", report.summary.total_public_repositories),
        ),
        (
            "Total Stars Accumulated",
            format!("{}", report.summary.total_stars_accumulated),
        ),
        (
            "Total Forks Accumulated",
            format!("{}", report.summary.total_forks_accumulated),
        ),
        (
            "Distinct Email Identities",
            format!("{}", report.summary.distinct_emails_observed),
        ),
        (
            "Public Keys Found",
            format!("{}", report.summary.public_keys_found),
        ),
        (
            "Fork Ratio",
            format!("{:.1}%", report.summary.fork_ratio * 100.0),
        ),
        (
            "Archived Ratio",
            format!("{:.1}%", report.summary.archived_ratio * 100.0),
        ),
    ];

    for (k, v) in summary_items {
        p1.extend_from_slice(
            format!(
                "BT /F2 9 Tf 0.3 0.35 0.4 rg 50 {y} Td ({}:) Tj ET\n",
                escape_pdf_text(k)
            )
            .as_bytes(),
        );
        p1.extend_from_slice(
            format!(
                "BT /F1 9 Tf 0.1 0.1 0.1 rg 210 {y} Td ({}) Tj ET\n",
                escape_pdf_text(&v)
            )
            .as_bytes(),
        );
        y -= 14;
    }

    // Section 2: Observed Public Identities
    y -= 10;
    p1.extend_from_slice(
        format!(
            "BT /F2 12 Tf 0.15 0.2 0.3 rg 40 {y} Td (2. PUBLIC IDENTITIES OBSERVED IN METADATA) Tj ET\n"
        )
        .as_bytes(),
    );
    y -= 5;
    p1.extend_from_slice(format!("0.2 0.4 0.8 RG 40 {y} 515 0.5 re S\n").as_bytes());
    y -= 16;

    // Table header
    p1.extend_from_slice(format!("0.94 0.95 0.96 rg 40 {y} 515 14 re f\n").as_bytes());
    p1.extend_from_slice(
        format!(
            "BT /F2 8 Tf 0.2 0.25 0.3 rg 45 {} Td (EMAIL) Tj 220 {} Td (COMMITS) Tj 270 {} Td (CLASSIFICATION) Tj 410 {} Td (VERIFIED) Tj ET\n",
            y + 3, y + 3, y + 3, y + 3
        )
        .as_bytes(),
    );
    y -= 14;

    if report.identities.is_empty() {
        p1.extend_from_slice(
            format!(
                "BT /F1 8 Tf 0.5 0.5 0.5 rg 45 {y} Td (No public commit or profile emails identified) Tj ET\n"
            )
            .as_bytes(),
        );
    } else {
        for id in report.identities.iter().take(9) {
            let email_esc = escape_pdf_text(&id.email);
            let class_esc = escape_pdf_text(id.email_type.as_str());
            let verified = if id.is_verified_signer { "Yes" } else { "No" };

            p1.extend_from_slice(
                format!(
                    "BT /F3 8 Tf 0.1 0.1 0.1 rg 45 {y} Td ({email_esc}) Tj ET\n\
                     BT /F1 8 Tf 0.2 0.2 0.2 rg 220 {y} Td ({commits}) Tj 270 {y} Td ({class_esc}) Tj 410 {y} Td ({verified}) Tj ET\n",
                    commits = id.commits_observed
                )
                .as_bytes(),
            );
            y -= 14;
        }
    }

    // Footnote on page 1
    p1.extend_from_slice(b"BT /F1 8 Tf 0.5 0.5 0.5 rg 40 65 Td (* Sourced strictly from public Git commit headers and GitHub public API endpoints.) Tj ET\n");
    p1.extend_from_slice(b"0.8 0.8 0.8 RG 40 50 515 0.5 re S\n");
    p1.extend_from_slice(b"BT /F1 8 Tf 0.5 0.5 0.5 rg 40 38 Td (Spoor by Voldigoade) Tj ET\n");
    p1.extend_from_slice(b"BT /F1 8 Tf 0.5 0.5 0.5 rg 505 38 Td (Page 1 of 2) Tj ET\n");

    // Page 2 Content
    let mut p2 = Vec::new();

    // Page 2 header band
    p2.extend_from_slice(b"0.08 0.1 0.14 rg 40 780 515 30 re f\n");
    p2.extend_from_slice(b"BT /F2 11 Tf 1 1 1 rg 50 790 Td (SPOOR - REPORT CONTINUATION) Tj ET\n");
    p2.extend_from_slice(
        format!(
            "BT /F1 9 Tf 0.7 0.8 0.9 rg 420 790 Td (Target: {}) Tj ET\n",
            escape_pdf_text(&report.target.display_name())
        )
        .as_bytes(),
    );

    let mut y2 = 750;

    // Section 3: Repositories Sample
    p2.extend_from_slice(
        format!(
            "BT /F2 12 Tf 0.15 0.2 0.3 rg 40 {y2} Td (3. PUBLIC REPOSITORIES (SAMPLE)) Tj ET\n"
        )
        .as_bytes(),
    );
    y2 -= 5;
    p2.extend_from_slice(format!("0.2 0.4 0.8 RG 40 {y2} 515 0.5 re S\n").as_bytes());
    y2 -= 16;

    p2.extend_from_slice(format!("0.94 0.95 0.96 rg 40 {y2} 515 14 re f\n").as_bytes());
    p2.extend_from_slice(
        format!(
            "BT /F2 8 Tf 0.2 0.25 0.3 rg 45 {} Td (REPOSITORY) Tj 240 {} Td (LANGUAGE) Tj 350 {} Td (STARS) Tj 420 {} Td (FORKS) Tj 480 {} Td (STATE) Tj ET\n",
            y2 + 3, y2 + 3, y2 + 3, y2 + 3, y2 + 3
        )
        .as_bytes(),
    );
    y2 -= 14;

    for r in report.repositories.iter().take(12) {
        let name_esc = escape_pdf_text(&r.name);
        let lang_esc = escape_pdf_text(r.primary_language.as_deref().unwrap_or("—"));
        let state = if r.is_archived {
            "Archived"
        } else if r.is_fork {
            "Fork"
        } else {
            "Active"
        };

        p2.extend_from_slice(
            format!(
                "BT /F2 8 Tf 0.1 0.2 0.4 rg 45 {y2} Td ({name_esc}) Tj ET\n\
                 BT /F1 8 Tf 0.2 0.2 0.2 rg 240 {y2} Td ({lang_esc}) Tj 350 {y2} Td ({stars}) Tj 420 {y2} Td ({forks}) Tj 480 {y2} Td ({state}) Tj ET\n",
                stars = r.stars,
                forks = r.forks,
            )
            .as_bytes(),
        );
        y2 -= 13;
    }

    // Section 4: Public Keys & Signing
    y2 -= 10;
    p2.extend_from_slice(
        format!(
            "BT /F2 12 Tf 0.15 0.2 0.3 rg 40 {y2} Td (4. PUBLIC KEYS & AUTHENTICATION MATERIAL) Tj ET\n"
        )
        .as_bytes(),
    );
    y2 -= 5;
    p2.extend_from_slice(format!("0.2 0.4 0.8 RG 40 {y2} 515 0.5 re S\n").as_bytes());
    y2 -= 14;

    if report.keys.is_empty() {
        p2.extend_from_slice(
            format!(
                "BT /F1 8 Tf 0.5 0.5 0.5 rg 45 {y2} Td (No public GPG or SSH keys registered) Tj ET\n"
            )
            .as_bytes(),
        );
        y2 -= 14;
    } else {
        for k in report.keys.iter().take(4) {
            let kid = escape_pdf_text(k.key_id.as_deref().unwrap_or("—"));
            let raw = escape_pdf_text(&k.raw_preview);
            p2.extend_from_slice(
                format!(
                    "BT /F2 8 Tf 0.2 0.2 0.2 rg 45 {y2} Td ([{}] ID: {kid}) Tj ET\n\
                     BT /F3 7 Tf 0.4 0.4 0.4 rg 170 {y2} Td ({raw}) Tj ET\n",
                    k.key_type
                )
                .as_bytes(),
            );
            y2 -= 12;
        }
    }

    // Section 5: Coverage, Provider Status & Limitations
    y2 -= 10;
    p2.extend_from_slice(
        format!(
            "BT /F2 12 Tf 0.15 0.2 0.3 rg 40 {y2} Td (5. COVERAGE & INHERENT LIMITATIONS) Tj ET\n"
        )
        .as_bytes(),
    );
    y2 -= 5;
    p2.extend_from_slice(format!("0.2 0.4 0.8 RG 40 {y2} 515 0.5 re S\n").as_bytes());
    y2 -= 14;

    p2.extend_from_slice(
        format!(
            "BT /F2 8 Tf 0.2 0.2 0.2 rg 45 {y2} Td (Coverage Accounting:) Tj ET\n\
             BT /F1 8 Tf 0.2 0.2 0.2 rg 160 {y2} Td ({} commits scanned across {}/{} public repositories) Tj ET\n",
            report.coverage.commits_scanned,
            report.coverage.repositories_scanned,
            report.coverage.repositories_total
        )
        .as_bytes(),
    );
    y2 -= 13;

    for lim in report.limitations.iter().take(4) {
        let lim_esc = escape_pdf_text(lim);
        p2.extend_from_slice(
            format!("BT /F1 7.5 Tf 0.35 0.4 0.45 rg 45 {y2} Td (* {lim_esc}) Tj ET\n").as_bytes(),
        );
        y2 -= 11;
    }

    // Page 2 footer
    p2.extend_from_slice(b"0.8 0.8 0.8 RG 40 50 515 0.5 re S\n");
    p2.extend_from_slice(b"BT /F1 8 Tf 0.5 0.5 0.5 rg 40 38 Td (Spoor by Voldigoade) Tj ET\n");
    p2.extend_from_slice(b"BT /F1 8 Tf 0.5 0.5 0.5 rg 505 38 Td (Page 2 of 2) Tj ET\n");

    // Compress streams with flate2
    fn compress_stream(data: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        let _ = encoder.write_all(data);
        encoder.finish().unwrap_or_else(|_| data.to_vec())
    }

    let p1_comp = compress_stream(&p1);
    let p2_comp = compress_stream(&p2);

    // 1: Catalog
    pdf.add_object(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());

    // 2: Pages tree
    pdf.add_object(
        format!(
            "<< /Type /Pages /Kids [ 6 0 R 8 0 R ] /Count {page_count} /MediaBox [ 0 0 595.28 841.89 ] >>"
        )
        .as_bytes()
        .to_vec(),
    );

    // 3: Font F1
    pdf.add_object(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    // 4: Font F2
    pdf.add_object(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_vec());
    // 5: Font F3
    pdf.add_object(b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_vec());

    // 6: Page 1
    pdf.add_object(
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> /Contents 7 0 R >>"
            .to_vec(),
    );

    // 7: Contents Page 1
    let mut c1 = format!(
        "<< /Filter /FlateDecode /Length {} >>\nstream\n",
        p1_comp.len()
    )
    .into_bytes();
    c1.extend_from_slice(&p1_comp);
    c1.extend_from_slice(b"\nendstream");
    pdf.add_object(c1);

    // 8: Page 2
    pdf.add_object(
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> /Contents 9 0 R >>"
            .to_vec(),
    );

    // 9: Contents Page 2
    let mut c2 = format!(
        "<< /Filter /FlateDecode /Length {} >>\nstream\n",
        p2_comp.len()
    )
    .into_bytes();
    c2.extend_from_slice(&p2_comp);
    c2.extend_from_slice(b"\nendstream");
    pdf.add_object(c2);

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
}
