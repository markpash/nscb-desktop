use regex::Regex;

pub fn sanitize_output_filename(name: &str) -> String {
    let (stem, ext) = if let Some((s, e)) = name.rsplit_once('.') {
        (s.to_string(), format!(".{}", e))
    } else {
        (name.to_string(), String::new())
    };
    let mut out = stem;
    // Mirror squirrel.py cleanup used for generated output names.
    out = Regex::new(r"[\/\\:\*\?]+")
        .unwrap()
        .replace_all(&out, "")
        .to_string();
    out = Regex::new(r#"[™©®`~\^´ªº¢#£€¥$ƒ±¬½¼♡«»•²‰œæÆ³☆<>|]"#)
        .unwrap()
        .replace_all(&out, "")
        .to_string();

    let translits = [
        ("Ⅰ", "I"),
        ("Ⅱ", "II"),
        ("Ⅲ", "III"),
        ("Ⅳ", "IV"),
        ("Ⅴ", "V"),
        ("Ⅵ", "VI"),
        ("Ⅶ", "VII"),
        ("Ⅷ", "VIII"),
        ("Ⅸ", "IX"),
        ("Ⅹ", "X"),
        ("Ⅺ", "XI"),
        ("Ⅻ", "XII"),
        ("Ⅼ", "L"),
        ("Ⅽ", "C"),
        ("Ⅾ", "D"),
        ("Ⅿ", "M"),
        ("—", "-"),
        ("√", "Root"),
        ("à", "a"),
        ("â", "a"),
        ("á", "a"),
        ("@", "a"),
        ("ä", "a"),
        ("å", "a"),
        ("À", "A"),
        ("Â", "A"),
        ("Á", "A"),
        ("Ä", "A"),
        ("Å", "A"),
        ("è", "e"),
        ("ê", "e"),
        ("é", "e"),
        ("ë", "e"),
        ("È", "E"),
        ("Ê", "E"),
        ("É", "E"),
        ("Ë", "E"),
        ("ì", "i"),
        ("î", "i"),
        ("í", "i"),
        ("ï", "i"),
        ("Ì", "I"),
        ("Î", "I"),
        ("Í", "I"),
        ("Ï", "I"),
        ("ò", "o"),
        ("ô", "o"),
        ("ó", "o"),
        ("ö", "o"),
        ("ø", "o"),
        ("Ò", "O"),
        ("Ô", "O"),
        ("Ó", "O"),
        ("Ö", "O"),
        ("Ø", "O"),
        ("ù", "u"),
        ("û", "u"),
        ("ú", "u"),
        ("ü", "u"),
        ("Ù", "U"),
        ("Û", "U"),
        ("Ú", "U"),
        ("Ü", "U"),
        ("’", "'"),
        ("“", "\""),
        ("”", "\""),
    ];
    for (from, to) in translits {
        out = out.replace(from, to);
    }

    out = Regex::new(r" {3,}")
        .unwrap()
        .replace_all(&out, " ")
        .to_string();
    out = out.replace("( ", "(");
    out = out.replace(" )", ")");
    out = out.replace("[ ", "[");
    out = out.replace(" ]", "]");
    out = out.replace("[ (", "[(");
    out = out.replace(") ]", ")]");
    out = out.replace("[]", "");
    out = out.replace("()", "");
    out = out.replace("\" ", "\"");
    out = out.replace(" \"", "\"");
    out = out.replace(" !", "!");
    out = out.replace(" ?", "?");
    out = out.replace("  ", " ");
    out = out.replace("  ", " ");
    out = out.replace('"', "");
    out = out.replace(")", ") ");
    out = out.replace("]", "] ");
    out = out.replace("[ (", "[(");
    out = out.replace(") ]", ")]");
    out = out.replace("  ", " ");
    out = out.trim_end().to_string();

    if out.is_empty() {
        if ext.is_empty() {
            "merged.nsp".to_string()
        } else {
            format!("merged{}", ext)
        }
    } else {
        format!("{}{}", out, ext)
    }
}

pub fn sanitize_python_split_title(name: &str) -> String {
    Regex::new(r#"[/\\:\*\?!"<>|\.\s™©®()~]+"#)
        .unwrap()
        .replace_all(name, " ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{sanitize_output_filename, sanitize_python_split_title};

    #[test]
    fn sanitize_output_filename_regression_removes_windows_unsafe_chars() {
        let out = sanitize_output_filename("Hollow Knight: Silksong [010013C00E930000].xci");
        assert!(
            !out.contains(':'),
            "sanitized output must not contain ':'; got {}",
            out
        );
        assert_eq!(
            out,
            "Hollow Knight Silksong [010013C00E930000].xci".to_string()
        );
    }

    #[test]
    fn sanitize_python_split_title_regression_replaces_unsafe_chars_with_spaces() {
        let out = sanitize_python_split_title("Ghost Master: Resurrection/Trial!.~");
        assert_eq!(out, "Ghost Master Resurrection Trial");
    }
}
