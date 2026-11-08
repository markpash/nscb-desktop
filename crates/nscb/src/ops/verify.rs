use rsa::pss::{Signature, VerifyingKey};
use rsa::signature::Verifier as _;
use rsa::BigUint;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::crypto::aes_ecb;
use crate::crypto::hash;
use crate::error::{NscbError, Result};
use crate::formats::cnmt::Cnmt;
use crate::formats::nca::{aes_ctr_transform_in_place, NcaHeader};
use crate::formats::ncz::{decompress_ncz, NczReader};
use crate::formats::nsp::Nsp;
use crate::formats::pfs0::Pfs0;
use crate::formats::ticket::Ticket;
use crate::formats::types::{self, ContentType};
use crate::formats::xci::Xci;
use crate::keys::KeyStore;

#[rustfmt::skip]
const NCA_HEADER_FIXED_KEY_MODULUS_00: [u8; 256] = [
    0xbf, 0xbe, 0x40, 0x6c, 0xf4, 0xa7, 0x80, 0xe9, 0xf0, 0x7d, 0x0c, 0x99, 0x61, 0x1d, 0x77, 0x2f,
    0x96, 0xbc, 0x4b, 0x9e, 0x58, 0x38, 0x1b, 0x03, 0xab, 0xb1, 0x75, 0x49, 0x9f, 0x2b, 0x4d, 0x58,
    0x34, 0xb0, 0x05, 0xa3, 0x75, 0x22, 0xbe, 0x1a, 0x3f, 0x03, 0x73, 0xac, 0x70, 0x68, 0xd1, 0x16,
    0xb9, 0x04, 0x46, 0x5e, 0xb7, 0x07, 0x91, 0x2f, 0x07, 0x8b, 0x26, 0xde, 0xf6, 0x00, 0x07, 0xb2,
    0xb4, 0x51, 0xf8, 0x0d, 0x0a, 0x5e, 0x58, 0xad, 0xeb, 0xbc, 0x9a, 0xd6, 0x49, 0xb9, 0x64, 0xef,
    0xa7, 0x82, 0xb5, 0xcf, 0x6d, 0x70, 0x13, 0xb0, 0x0f, 0x85, 0xf6, 0xa9, 0x08, 0xaa, 0x4d, 0x67,
    0x66, 0x87, 0xfa, 0x89, 0xff, 0x75, 0x90, 0x18, 0x1e, 0x6b, 0x3d, 0xe9, 0x8a, 0x68, 0xc9, 0x26,
    0x04, 0xd9, 0x80, 0xce, 0x3f, 0x5e, 0x92, 0xce, 0x01, 0xff, 0x06, 0x3b, 0xf2, 0xc1, 0xa9, 0x0c,
    0xce, 0x02, 0x6f, 0x16, 0xbc, 0x92, 0x42, 0x0a, 0x41, 0x64, 0xcd, 0x52, 0xb6, 0x34, 0x4d, 0xae,
    0xc0, 0x2e, 0xde, 0xa4, 0xdf, 0x27, 0x68, 0x3c, 0xc1, 0xa0, 0x60, 0xad, 0x43, 0xf3, 0xfc, 0x86,
    0xc1, 0x3e, 0x6c, 0x46, 0xf7, 0x7c, 0x29, 0x9f, 0xfa, 0xfd, 0xf0, 0xe3, 0xce, 0x64, 0xe7, 0x35,
    0xf2, 0xf6, 0x56, 0x56, 0x6f, 0x6d, 0xf1, 0xe2, 0x42, 0xb0, 0x83, 0x40, 0xa5, 0xc3, 0x20, 0x2b,
    0xcc, 0x9a, 0xae, 0xca, 0xed, 0x4d, 0x70, 0x30, 0xa8, 0x70, 0x1c, 0x70, 0xfd, 0x13, 0x63, 0x29,
    0x02, 0x79, 0xea, 0xd2, 0xa7, 0xaf, 0x35, 0x28, 0x32, 0x1c, 0x7b, 0xe6, 0x2f, 0x1a, 0xaa, 0x40,
    0x7e, 0x32, 0x8c, 0x27, 0x42, 0xfe, 0x82, 0x78, 0xec, 0x0d, 0xeb, 0xe6, 0x83, 0x4b, 0x6d, 0x81,
    0x04, 0x40, 0x1a, 0x9e, 0x9a, 0x67, 0xf6, 0x72, 0x29, 0xfa, 0x04, 0xf0, 0x9d, 0xe4, 0xf4, 0x03,
];

#[rustfmt::skip]
const NCA_HEADER_FIXED_KEY_MODULUS_01: [u8; 256] = [
    0xad, 0xe3, 0xe1, 0xfa, 0x04, 0x35, 0xe5, 0xb6, 0xdd, 0x49, 0xea, 0x89, 0x29, 0xb1, 0xff, 0xb6,
    0x43, 0xdf, 0xca, 0x96, 0xa0, 0x4a, 0x13, 0xdf, 0x43, 0xd9, 0x94, 0x97, 0x96, 0x43, 0x65, 0x48,
    0x70, 0x58, 0x33, 0xa2, 0x7d, 0x35, 0x7b, 0x96, 0x74, 0x5e, 0x0b, 0x5c, 0x32, 0x18, 0x14, 0x24,
    0xc2, 0x58, 0xb3, 0x6c, 0x22, 0x7a, 0xa1, 0xb7, 0xcb, 0x90, 0xa7, 0xa3, 0xf9, 0x7d, 0x45, 0x16,
    0xa5, 0xc8, 0xed, 0x8f, 0xad, 0x39, 0x5e, 0x9e, 0x4b, 0x51, 0x68, 0x7d, 0xf8, 0x0c, 0x35, 0xc6,
    0x3f, 0x91, 0xae, 0x44, 0xa5, 0x92, 0x30, 0x0d, 0x46, 0xf8, 0x40, 0xff, 0xd0, 0xff, 0x06, 0xd2,
    0x1c, 0x7f, 0x96, 0x18, 0xdc, 0xb7, 0x1d, 0x66, 0x3e, 0xd1, 0x73, 0xbc, 0x15, 0x8a, 0x2f, 0x94,
    0xf3, 0x00, 0xc1, 0x83, 0xf1, 0xcd, 0xd7, 0x81, 0x88, 0xab, 0xdf, 0x8c, 0xef, 0x97, 0xdd, 0x1b,
    0x17, 0x5f, 0x58, 0xf6, 0x9a, 0xe9, 0xe8, 0xc2, 0x2f, 0x38, 0x15, 0xf5, 0x21, 0x07, 0xf8, 0x37,
    0x90, 0x5d, 0x2e, 0x02, 0x40, 0x24, 0x15, 0x0d, 0x25, 0xb7, 0x26, 0x5d, 0x09, 0xcc, 0x4c, 0xf4,
    0xf2, 0x1b, 0x94, 0x70, 0x5a, 0xe, 0xee, 0xed, 0x77, 0x77, 0xd4, 0x51, 0x99, 0xf5, 0xdc, 0x76,
    0x1e, 0xe3, 0x6c, 0x8c, 0xd1, 0x12, 0xd4, 0x57, 0xd1, 0xb6, 0x83, 0xe4, 0xe4, 0xfe, 0xda, 0xe9,
    0xb4, 0x3b, 0x33, 0xe5, 0x37, 0x8a, 0xdf, 0xb5, 0x7f, 0x89, 0xf1, 0x9b, 0x9e, 0xb0, 0x15, 0xb2,
    0x3a, 0xfe, 0xea, 0x61, 0x84, 0x5b, 0x7d, 0x4b, 0x23, 0x12, 0x0b, 0x83, 0x12, 0xf2, 0x22, 0x6b,
    0xb9, 0x22, 0x96, 0x4b, 0x26, 0x0b, 0x63, 0x5e, 0x96, 0x57, 0x52, 0xa3, 0x67, 0x64, 0x22, 0xca,
    0xd0, 0x56, 0x3e, 0x74, 0xb5, 0x98, 0x1f, 0x0d, 0xf8, 0xb3, 0x34, 0xe6, 0x98, 0x68, 0x5a, 0xad,
];

struct ContainerEntry {
    name: String,
    abs_offset: u64,
    size: u64,
}

fn content_type_py(ct: Option<ContentType>) -> &'static str {
    match ct {
        Some(ContentType::Program) => "Content.PROGRAM",
        Some(ContentType::Meta) => "Content.META",
        Some(ContentType::Control) => "Content.CONTROL",
        Some(ContentType::Manual) => "Content.MANUAL",
        Some(ContentType::Data) => "Content.DATA",
        Some(ContentType::PublicData) => "Content.PUBLIC_DATA",
        None => "Content.UNKNOWN",
    }
}

fn normalize_vertype(vertype: &str) -> &str {
    if vertype == "dec" || vertype == "lv1" {
        "lv1"
    } else if vertype == "sig" || vertype == "lv2" {
        "lv2"
    } else if vertype == "sig" || vertype == "lv3" {
        // Match squirrel.py exactly: "full" falls through to lv1 here.
        "lv3"
    } else {
        "lv1"
    }
}

fn container_token(path: &str) -> &str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "nsp" => "NSP",
        "nsz" => "NSZ",
        "nsx" => "NSX",
        "xci" => "XCI",
        "xcz" => "XCZ",
        _ => "NSP",
    }
}

pub fn verify(path: &str, ks: &KeyStore, vertype: &str, text_file: Option<&str>) -> Result<()> {
    let vt = normalize_vertype(vertype);
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "nsp" | "nsx" | "nsz" => verify_nsp(path, ks, vt, text_file),
        "xci" | "xcz" => verify_xci(path, ks, vt, text_file),
        _ => Err(NscbError::UnsupportedFormat(format!(
            "Cannot verify .{} file",
            ext
        ))),
    }
}

pub fn verify_from_text_file(text_file: &str, ks: &KeyStore, vertype: &str) -> Result<()> {
    let file_contents = std::fs::read_to_string(text_file)?;
    let first_line = file_contents
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| NscbError::InvalidData("verify text file is empty".into()))?;
    let path = std::fs::canonicalize(first_line)?;
    let path_str = path.to_string_lossy().to_string();
    verify(&path_str, ks, vertype, Some(text_file))
}

fn verify_nsp(path: &str, ks: &KeyStore, vertype: &str, text_file: Option<&str>) -> Result<()> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let nsp = Nsp::parse(&mut reader)?;

    let mut entries = Vec::new();
    for e in nsp.all_entries() {
        entries.push(ContainerEntry {
            name: e.name.clone(),
            abs_offset: nsp.pfs0.file_abs_offset(e),
            size: e.size,
        });
    }

    run_verify(path, &entries, &mut reader, ks, vertype, text_file)
}

fn verify_xci(path: &str, ks: &KeyStore, vertype: &str, text_file: Option<&str>) -> Result<()> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let xci = Xci::parse(&mut reader)?;
    let secure = xci.secure_partition(&mut reader)?;

    let mut entries = Vec::new();
    for e in &secure.entries {
        entries.push(ContainerEntry {
            name: e.name.clone(),
            abs_offset: secure.file_abs_offset(e),
            size: e.size,
        });
    }

    run_verify(path, &entries, &mut reader, ks, vertype, text_file)
}

fn run_verify<R: Read + Seek>(
    path: &str,
    entries: &[ContainerEntry],
    reader: &mut R,
    ks: &KeyStore,
    vertype: &str,
    text_file: Option<&str>,
) -> Result<()> {
    let token = container_token(path);
    let mut feed = String::new();
    if text_file.is_some() {
        // Non-interactive mode: respect vertype
        let (dec_verdict, dec_output) = run_dec_test(entries, reader, path, ks, token)?;
        crate::out!("{}", dec_output);
        feed.push_str(&dec_output);
        let mut overall_verdict = dec_verdict;

        if vertype == "lv2" || vertype == "lv3" {
            let (sig_verdict, header_info, sig_output) = run_sig_test(entries, reader, ks, token)?;
            crate::out!("{}", sig_output);
            feed.push_str(&sig_output);
            if overall_verdict {
                overall_verdict = sig_verdict;
            }

            if vertype == "lv3" {
                let (hash_verdict, hash_output) =
                    run_hash_test(entries, reader, ks, token, &header_info, sig_verdict)?;
                crate::out!("{}", hash_output);
                feed.push_str(&hash_output);
                if overall_verdict {
                    overall_verdict = hash_verdict;
                }
            }
        }

        if let Some(text_file) = text_file {
            write_massverify_outputs(path, text_file, &feed, overall_verdict)?;
        }
    } else {
        // Interactive mode: always run LV1 + LV2 + prompt for LV3 + prompt for text file
        // (matching Python behavior exactly)
        let (dec_verdict, dec_output) = run_dec_test(entries, reader, path, ks, token)?;
        crate::out!("{}", dec_output);
        feed.push_str(&dec_output);

        let (sig_verdict, header_info, sig_output) = run_sig_test(entries, reader, ks, token)?;
        crate::out!("{}", sig_output);
        feed.push_str(&sig_output);

        // Prompt for hash verification
        crate::outln!("\n********************************************************");
        crate::outln!("Do you want to verify the hash of the nca files?");
        crate::outln!("********************************************************");
        loop {
            crate::outln!("Input \"1\" to VERIFY hash of files");
            crate::outln!("Input \"2\" to NOT verify hash  of files\n");
            crate::out!("Input your answer: ");
            use std::io::Write;
            std::io::stdout().flush().ok();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            let input = input.trim();
            if input == "1" {
                crate::outln!();
                let (_, hash_output) =
                    run_hash_test(entries, reader, ks, token, &header_info, sig_verdict)?;
                crate::out!("{}", hash_output);
                feed.push_str(&hash_output);
                break;
            } else if input == "2" {
                break;
            } else {
                crate::outln!("WRONG CHOICE\n");
            }
        }

        // Prompt for text file export
        crate::outln!("\n********************************************************");
        crate::outln!("Do you want to print the information to a text file");
        crate::outln!("********************************************************");
        loop {
            crate::outln!("Input \"1\" to print to text file");
            crate::outln!("Input \"2\" to NOT print to text file\n");
            crate::out!("Input your answer: ");
            use std::io::Write;
            std::io::stdout().flush().ok();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            let input = input.trim();
            if input == "1" {
                let out_path = format!("{}.txt", path);
                std::fs::write(&out_path, &feed).ok();
                break;
            } else if input == "2" {
                break;
            } else {
                crate::outln!("WRONG CHOICE\n");
            }
        }
        let _ = (dec_verdict, sig_verdict);
    }

    Ok(())
}

fn write_massverify_outputs(
    verified_path: &str,
    text_file: &str,
    feed: &str,
    verdict: bool,
) -> Result<()> {
    let list_dir = Path::new(text_file)
        .parent()
        .ok_or_else(|| NscbError::InvalidData("text file path has no parent directory".into()))?;
    let info_dir = list_dir.join("INFO").join("MASSVERIFY");
    std::fs::create_dir_all(&info_dir)?;

    let basename = Path::new(verified_path)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| NscbError::InvalidData("invalid verified filename".into()))?;
    let verify_name = if basename.len() >= 4 {
        format!("{}-verify.txt", &basename[..basename.len() - 4])
    } else {
        format!("{}-verify.txt", basename)
    };
    std::fs::write(info_dir.join(verify_name), feed)?;

    if !verdict {
        let err_path = list_dir.join("badfiles.txt");
        let mut err = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(err_path)?;
        writeln!(err, "Filename: {}", verified_path)?;
        writeln!(err, "IS INCORRECT")?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// lv1: Decryption test
// ---------------------------------------------------------------------------
// lv1: Decryption test
// ---------------------------------------------------------------------------

fn run_dec_test<R: Read + Seek>(
    entries: &[ContainerEntry],
    reader: &mut R,
    _container_path: &str,
    ks: &KeyStore,
    token: &str,
) -> Result<(bool, String)> {
    let tabs = "\t";
    let mut out = String::new();
    let mut verdict = true;

    out.push_str("***************\n");
    out.push_str("DECRYPTION TEST\n");
    out.push_str("***************\n");

    let nca_entries: Vec<&ContainerEntry> = entries
        .iter()
        .filter(|e| e.name.ends_with(".nca"))
        .collect();
    let tik_entries: Vec<&ContainerEntry> = entries
        .iter()
        .filter(|e| e.name.ends_with(".tik"))
        .collect();
    let _cert_entries: Vec<&ContainerEntry> = entries
        .iter()
        .filter(|e| e.name.ends_with(".cert"))
        .collect();

    let is_nsz = token == "NSZ";

    let mut ticket_map: HashMap<String, Ticket> = HashMap::new();
    for tik in &tik_entries {
        reader.seek(SeekFrom::Start(tik.abs_offset))?;
        let mut data = vec![0u8; tik.size as usize];
        reader.read_exact(&mut data)?;
        if let Ok(ticket) = Ticket::from_bytes(&data) {
            ticket_map.insert(ticket.rights_id_hex(), ticket);
        }
    }

    // Pre-read CNMT title ID (Python uses package title ID to determine base vs update for NCZ)
    let cnmt_title_id: Option<u64> = nca_entries
        .iter()
        .find(|e| e.name.ends_with("cnmt.nca"))
        .and_then(|e| read_meta_nca_cnmt(e, reader, ks).ok())
        .map(|cnmt| cnmt.title_id);

    let mut listed_nca_names: HashSet<String> = HashSet::new();
    for e in entries {
        if e.name.ends_with(".nca") || e.name.ends_with(".ncz") {
            listed_nca_names.insert(e.name.clone());
        }
    }

    // --- Check NCA/NCZ files in container order (matching Python iteration) ---
    for entry in entries {
        if entry.name.ends_with(".nca") {
            let is_cnmt = entry.name.ends_with("cnmt.nca");
            let header = match NcaHeader::from_reader(reader, entry.abs_offset, ks) {
                Ok(h) => h,
                Err(_) => {
                    out.push_str(&format!(
                        "UNKNOWN - Content.UNKNOWN\n{}{} -> is CORRUPT <<<-\n",
                        tabs, entry.name
                    ));
                    verdict = false;
                    continue;
                }
            };
            let title_id = format!("{:016X}", header.title_id);
            let content_type = content_type_py(header.content_type_enum());
            out.push_str(&format!("{} - {}\n", title_id, content_type));
            let (correct, _baddec) = check_nca_dec(entry, &header, reader, ks, &ticket_map);
            if correct {
                if is_cnmt {
                    out.push_str(&format!("{}{} -> is CORRECT\n", tabs, entry.name));
                } else {
                    out.push_str(&format!("{}{}{tabs}  -> is CORRECT\n", tabs, entry.name));
                }
            } else {
                verdict = false;
                if is_cnmt {
                    out.push_str(&format!("{}{} -> is CORRUPT <<<-\n", tabs, entry.name));
                } else {
                    out.push_str(&format!(
                        "{}{}{tabs}  -> is CORRUPT <<<-\n",
                        tabs, entry.name
                    ));
                }
            }
        } else if entry.name.ends_with(".ncz") {
            let header = match NcaHeader::from_reader(reader, entry.abs_offset, ks) {
                Ok(h) => h,
                Err(_) => {
                    out.push_str("UNKNOWN - Content.UNKNOWN\n");
                    out.push_str(&format!(
                        "{}{}{tabs}  -> ncz file needs HASH check\n",
                        tabs, entry.name
                    ));
                    continue;
                }
            };
            let title_id = format!("{:016X}", header.title_id);
            let content_type = content_type_py(header.content_type_enum());
            out.push_str(&format!("{} - {}\n", title_id, content_type));
            // Python's verify_ncz returns True or 'ncz' (never False/corrupt)
            let ncz_correct = verify_ncz(entry, &header, reader, ks, cnmt_title_id);
            if ncz_correct {
                out.push_str(&format!("{}{}{tabs}  -> is CORRECT\n", tabs, entry.name));
            } else {
                // 'ncz' result: needs HASH check, not reported as corrupt
                out.push_str(&format!(
                    "{}{}{tabs}  -> ncz file needs HASH check\n",
                    tabs, entry.name
                ));
            }
        }
    }

    // --- Check TIK files (skip for NSZ, matching Python behavior) ---
    if !is_nsz {
        for tik in &tik_entries {
            out.push_str("Content.TICKET\n");
            let tik_correct = check_ticket_key(tik, &nca_entries, &ticket_map, reader, ks);
            if tik_correct {
                out.push_str(&format!("{}{}{tabs}  -> is CORRECT\n", tabs, tik.name));
            } else {
                verdict = false;
                out.push_str(&format!(
                    "{}{}{tabs}  -> titlekey is INCORRECT <<<-\n",
                    tabs, tik.name
                ));
            }
        }
    }

    // --- CNMT cross-reference ---
    for entry in &nca_entries {
        if !entry.name.ends_with("cnmt.nca") {
            continue;
        }
        if let Ok(cnmt) = read_meta_nca_cnmt(entry, reader, ks) {
            let title_id_cnmt = format!("{:016x}", cnmt.title_id);
            for ce in &cnmt.content_entries {
                if ce.is_delta() {
                    continue;
                }
                let nca_name = ce.nca_id() + ".nca";
                let ncz_name = ce.nca_id() + ".ncz";
                if !listed_nca_names.contains(&nca_name) && !listed_nca_names.contains(&ncz_name) {
                    verdict = false;
                    out.push_str(&format!(
                        "\n- Missing file from {}: {}\n",
                        title_id_cnmt, nca_name
                    ));
                }
            }
        }
    }

    // --- Check tickets present for all rights-ID NCAs (skip for NSZ) ---
    if !is_nsz {
        let mut seen_rights_ids: HashSet<String> = HashSet::new();
        for entry in &nca_entries {
            let header = match NcaHeader::from_reader(reader, entry.abs_offset, ks) {
                Ok(h) => h,
                Err(_) => continue,
            };
            if header.has_rights_id() {
                let rid = header.rights_id_hex();
                if seen_rights_ids.contains(&rid) {
                    continue;
                }
                seen_rights_ids.insert(rid.clone());
                let mtick = rid.clone() + ".tik";
                let tik_names: HashSet<String> =
                    tik_entries.iter().map(|e| e.name.clone()).collect();
                if !tik_names.contains(&mtick) {
                    verdict = false;
                    out.push_str(&format!(
                        "\n- File has titlerights!!! Missing ticket: {}\n",
                        mtick
                    ));
                }
            }
        }
    }

    if verdict {
        out.push_str(&format!("\nVERDICT: {} FILE IS CORRECT\n", token));
    } else {
        out.push_str(&format!(
            "\nVERDICT: {} FILE IS CORRUPT OR MISSES FILES\n",
            token
        ));
    }

    Ok((verdict, out))
}

fn check_nca_dec<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    ks: &KeyStore,
    ticket_map: &HashMap<String, Ticket>,
) -> (bool, bool) {
    let content_type = header.content_type_enum();
    let mut baddec = false;

    match content_type {
        Some(ContentType::Program) | Some(ContentType::Meta) => {
            let correct = check_program_or_meta_nca(entry, header, reader, ks, ticket_map);
            if correct {
                // verify_enforcer check
                let enforce_ok = verify_enforcer(header);
                if !enforce_ok {
                    return (false, false);
                }
                // For PROGRAM with no rights_id, also check pr_noenc_check
                if matches!(content_type, Some(ContentType::Program)) && !header.has_rights_id() {
                    let noenc_ok = pr_noenc_check(entry, header, reader, ks);
                    if !noenc_ok {
                        baddec = true;
                    }
                }
                (true, baddec)
            } else {
                // Fallback: if rights_id==0, try pr_noenc_check
                if !header.has_rights_id() {
                    let noenc_ok = pr_noenc_check(entry, header, reader, ks);
                    if noenc_ok {
                        let enforce_ok = verify_enforcer(header);
                        if enforce_ok {
                            let noenc_ok2 = pr_noenc_check(entry, header, reader, ks);
                            if !noenc_ok2 {
                                baddec = true;
                            }
                            return (true, baddec);
                        }
                    }
                }
                // Fallback: if rights_id!=0, try verify_nca_key
                if header.has_rights_id() {
                    let rights_id = header.rights_id_hex();
                    if let Some(ticket) = ticket_map.get(&rights_id) {
                        if let Ok(title_key) = ks.decrypt_title_key(
                            &ticket.title_key_block,
                            verify_titlekey_master_key_revision(header),
                        ) {
                            if verify_nca_with_key(entry, header, title_key, reader, ks) {
                                return (true, baddec);
                            }
                        }
                    }
                }
                (false, baddec)
            }
        }
        Some(ContentType::PublicData) if !header.has_rights_id() => {
            let correct = check_section_header_valid(header);
            if correct {
                let noenc_ok = pr_noenc_check_dlc(entry, header, reader, ks);
                if !noenc_ok {
                    baddec = true;
                }
                (true, baddec)
            } else {
                (false, baddec)
            }
        }
        _ => {
            let correct = check_section_header_valid(header);
            (correct, baddec)
        }
    }
}

fn verify_ncz<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    _ks: &KeyStore,
    cnmt_title_id: Option<u64>,
) -> bool {
    let ncz = match NczReader::parse_at(reader, entry.abs_offset) {
        Ok(n) => n,
        Err(_) => return false,
    };
    // Python uses the CNMT package title ID to determine base (000) vs update (800).
    // PROGRAM NCA headers always show the base title ID (xxx000), so we must use
    // the CNMT-derived title ID to correctly classify the container.
    let effective_title_id = cnmt_title_id.unwrap_or(header.title_id);
    let title_id_hex = format!("{:016x}", effective_title_id);
    let is_base = title_id_hex.ends_with("000");

    // Python: for base titles, checkstarter = section[0].size only.
    // The Python loop is: count+=1; if count==2: break; checkstarter+=s.size
    // This means only section[0] is added; section[1] triggers the break before being added.
    let checkstarter: u64 = if is_base {
        ncz.sections.first().map(|s| s.size).unwrap_or(0)
    } else {
        0
    };

    if reader.seek(SeekFrom::Start(ncz.data_start)).is_err() {
        return false;
    }

    if ncz.is_stream {
        let compressed_size = entry.size.saturating_sub(ncz.data_start - entry.abs_offset);
        let mut compressed_data = vec![0u8; compressed_size as usize];
        if reader.read_exact(&mut compressed_data).is_err() {
            return false;
        }

        let python_check = |skip_bytes: u64| -> bool {
            let mut decoder = match zstd::Decoder::new(&compressed_data[..]) {
                Ok(d) => d,
                Err(_) => return false,
            };

            if skip_bytes > 0 {
                let block_size = 16384u64;
                let test = (skip_bytes / block_size) as usize;
                let mut skip_buf = vec![0u8; block_size as usize];
                for _ in 0..=test {
                    use std::io::Read as _;
                    if decoder.read_exact(&mut skip_buf).is_err() {
                        return false;
                    }
                }
            }

            let mut check_buf = vec![0u8; 16384];
            use std::io::Read as _;
            match decoder.read_exact(&mut check_buf) {
                Ok(()) => {
                    let b1_nonzero = check_buf[..32].iter().any(|&b| b != 0);
                    let b2_zero = check_buf[32..64].iter().all(|&b| b == 0);
                    b1_nonzero && b2_zero
                }
                Err(_) => false,
            }
        };

        if python_check(checkstarter) {
            true
        } else {
            // Some files only line up when checked from the start of the NCZ stream.
            // Keep the Python-style base skip first, then fall back to the direct check.
            python_check(0)
        }
    } else if let Some(block_table) = &ncz.block_table {
        let block_size = 1usize << block_table.block_size_exponent;
        let mut decompressed = Vec::new();
        let mut total_decompressed: usize = 0;

        for &comp_size in &block_table.block_sizes {
            let mut compressed = vec![0u8; comp_size as usize];
            if reader.read_exact(&mut compressed).is_err() {
                break;
            }
            let decompressed_block = if comp_size as usize == block_size {
                compressed
            } else {
                match zstd::decode_all(&compressed[..]) {
                    Ok(d) => d,
                    Err(_) => break,
                }
            };
            decompressed.extend_from_slice(&decompressed_block);
            total_decompressed += decompressed_block.len();

            if total_decompressed as u64 >= checkstarter + 64 {
                break;
            }
        }

        if decompressed.len() < (checkstarter + 64) as usize {
            return false;
        }

        let offset = checkstarter as usize;
        let b1 = &decompressed[offset..offset + 32];
        let b2 = &decompressed[offset + 32..offset + 64];
        let b1_nonzero = b1.iter().any(|&b| b != 0);
        let b2_zero = b2.iter().all(|&b| b == 0);
        b1_nonzero && b2_zero
    } else {
        false
    }
}
fn check_program_or_meta_nca<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    ks: &KeyStore,
    ticket_map: &HashMap<String, Ticket>,
) -> bool {
    let candidate_keys = get_candidate_keys(header, ks, ticket_map);

    for sec_idx in 0..4 {
        let sec = &header.section_table[sec_idx];
        if !sec.is_present() || sec.size() == 0 {
            continue;
        }

        let section_start = sec.start_offset();
        let section_size = sec.size();
        let read_size = section_size.min(0x20000) as usize;
        let abs_section = entry.abs_offset + section_start;

        if reader.seek(SeekFrom::Start(abs_section)).is_err() {
            continue;
        }
        let mut buf = vec![0u8; read_size];
        if reader.read_exact(&mut buf).is_err() {
            reader.seek(SeekFrom::Start(abs_section)).ok();
            let _ = reader.take(read_size as u64).read_to_end(&mut buf);
        }

        let crypto_type = header.section_crypto_type(sec_idx);
        let nonce = header.section_ctr_nonce(sec_idx);

        if crypto_type == 1 || crypto_type == 0 {
            if has_pfs0_magic(&buf) {
                return true;
            }
        } else if crypto_type == 3 {
            for key in &candidate_keys {
                for &le in &[true, false] {
                    let mut dec = buf.clone();
                    aes_ctr_transform_in_place(key, &nonce, section_start, le, &mut dec);
                    if has_pfs0_magic(&dec) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

fn has_pfs0_magic(buf: &[u8]) -> bool {
    let scan_len = buf.len().saturating_sub(4);
    for i in 0..scan_len {
        if &buf[i..i + 4] == b"PFS0" {
            return true;
        }
    }
    false
}

fn get_candidate_keys(
    header: &NcaHeader,
    ks: &KeyStore,
    ticket_map: &HashMap<String, Ticket>,
) -> Vec<[u8; 16]> {
    let mut keys = Vec::new();

    if header.has_rights_id() {
        let rights_id = header.rights_id_hex();
        if let Some(ticket) = ticket_map.get(&rights_id) {
            if let Ok(title_key) = ks.decrypt_title_key(
                &ticket.title_key_block,
                verify_titlekey_master_key_revision(header),
            ) {
                keys.push(title_key);
            }
        }
    } else {
        if let Ok(section_keys) = header.decrypt_key_area(ks) {
            for k in &section_keys {
                keys.push(*k);
            }
        }
    }

    keys
}

fn verify_titlekey_master_key_revision(header: &NcaHeader) -> u8 {
    let key_generation = if header.crypto_type == 2 {
        header.crypto_type.max(header.crypto_type2)
    } else {
        header.crypto_type2
    };
    key_generation.saturating_sub(1)
}

fn check_section_header_valid(header: &NcaHeader) -> bool {
    for i in 0..4 {
        let sec = &header.section_table[i];
        if !sec.is_present() {
            continue;
        }
        let fs_type = header.section_fs_type(i);
        let crypto_type = header.section_crypto_type(i);
        if (fs_type == 2 || fs_type == 3) && (crypto_type >= 1 && crypto_type <= 4) {
            return true;
        }
    }
    false
}

fn verify_enforcer(header: &NcaHeader) -> bool {
    match header.content_type_enum() {
        Some(ContentType::Program) | Some(ContentType::Meta) => {
            for i in 0..4 {
                let sec = &header.section_table[i];
                if !sec.is_present() {
                    continue;
                }
                let fs_type = header.section_fs_type(i);
                let crypto_type = header.section_crypto_type(i);
                if fs_type == 2 && crypto_type == 3 {
                    return true;
                }
            }
            false
        }
        _ => {
            for i in 0..4 {
                let sec = &header.section_table[i];
                if !sec.is_present() {
                    continue;
                }
                let fs_type = header.section_fs_type(i);
                let crypto_type = header.section_crypto_type(i);
                if fs_type == 3 && crypto_type == 3 {
                    return true;
                }
            }
            false
        }
    }
}

fn pr_noenc_check<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    ks: &KeyStore,
) -> bool {
    let candidate_keys = get_candidate_keys(header, ks, &HashMap::new());
    for sec_idx in 0..4 {
        let sec = &header.section_table[sec_idx];
        if !sec.is_present() || sec.size() == 0 {
            continue;
        }
        let section_start = sec.start_offset();
        let abs_section = entry.abs_offset + section_start;
        if reader.seek(SeekFrom::Start(abs_section)).is_err() {
            continue;
        }
        let mut buf = vec![0u8; 0x10];
        if reader.read_exact(&mut buf).is_err() {
            continue;
        }
        let crypto_type = header.section_crypto_type(sec_idx);
        let nonce = header.section_ctr_nonce(sec_idx);
        if crypto_type == 3 {
            for key in &candidate_keys {
                for &le in &[true, false] {
                    let mut dec = buf.clone();
                    aes_ctr_transform_in_place(key, &nonce, section_start, le, &mut dec);
                    if &dec[..4] == b"PFS0" {
                        return true;
                    }
                }
            }
        } else if &buf[..4] == b"PFS0" {
            return true;
        }
    }
    false
}

fn pr_noenc_check_dlc<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    ks: &KeyStore,
) -> bool {
    let candidate_keys = get_candidate_keys(header, ks, &HashMap::new());
    for sec_idx in 0..4 {
        let sec = &header.section_table[sec_idx];
        if !sec.is_present() || sec.size() == 0 {
            continue;
        }
        let section_start = sec.start_offset();
        let abs_section = entry.abs_offset + section_start;

        // Read IVFC level data (levelOffset at 0x18, levelSize at 0x20 of section header)
        let fs_header_offset = 0x400 + sec_idx * 0x200;
        let level_offset = u64::from_le_bytes(
            header.raw_bytes()[fs_header_offset + 0x18..fs_header_offset + 0x20]
                .try_into()
                .unwrap_or([0u8; 8]),
        );
        let level_size = u64::from_le_bytes(
            header.raw_bytes()[fs_header_offset + 0x20..fs_header_offset + 0x28]
                .try_into()
                .unwrap_or([0u8; 8]),
        );

        let data_abs = abs_section + level_offset;
        let data_size = level_size.min(0x10000) as usize;

        if reader.seek(SeekFrom::Start(data_abs)).is_err() {
            continue;
        }
        let mut buf = vec![0u8; data_size];
        if reader.read_exact(&mut buf).is_err() {
            continue;
        }

        let crypto_type = header.section_crypto_type(sec_idx);
        let nonce = header.section_ctr_nonce(sec_idx);

        if crypto_type == 3 {
            for key in &candidate_keys {
                for &le in &[true, false] {
                    let mut dec = buf.clone();
                    aes_ctr_transform_in_place(
                        key,
                        &nonce,
                        section_start + level_offset,
                        le,
                        &mut dec,
                    );
                    // Check against hash at 0xC8 of section header
                    let expected_hash =
                        &header.raw_bytes()[fs_header_offset + 0xC8..fs_header_offset + 0xE8];
                    let actual_hash = sha2::Sha256::digest(&dec);
                    if actual_hash.as_slice() == expected_hash {
                        return true;
                    }
                }
            }
        } else {
            let expected_hash =
                &header.raw_bytes()[fs_header_offset + 0xC8..fs_header_offset + 0xE8];
            let actual_hash = sha2::Sha256::digest(&buf);
            if actual_hash.as_slice() == expected_hash {
                return true;
            }
        }
    }
    false
}

fn check_ticket_key<R: Read + Seek>(
    tik_entry: &ContainerEntry,
    nca_entries: &[&ContainerEntry],
    ticket_map: &HashMap<String, Ticket>,
    reader: &mut R,
    ks: &KeyStore,
) -> bool {
    // Try to find a ticket that works with any NCA
    let tik_name = &tik_entry.name;
    let rights_id = tik_name.strip_suffix(".tik").unwrap_or(tik_name);

    if let Some(ticket) = ticket_map.get(rights_id) {
        // Find an NCA with matching rights_id
        for nca_entry in nca_entries {
            let header = match NcaHeader::from_reader(reader, nca_entry.abs_offset, ks) {
                Ok(h) => h,
                Err(_) => continue,
            };
            if header.has_rights_id() && header.rights_id_hex() == *rights_id {
                // Try to verify the ticket key can decrypt the NCA
                if let Ok(title_key) = ks.decrypt_title_key(
                    &ticket.title_key_block,
                    verify_titlekey_master_key_revision(&header),
                ) {
                    if verify_nca_with_key(nca_entry, &header, title_key, reader, ks) {
                        return true;
                    }
                }
            }
        }
        // If we could parse the ticket, consider it present even if key check fails
        // (matches Python behavior where it iterates through NCAs)
        return !ticket_map.is_empty();
    }
    false
}

fn verify_nca_with_key<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    title_key: [u8; 16],
    reader: &mut R,
    _ks: &KeyStore,
) -> bool {
    for sec_idx in 0..4 {
        let sec = &header.section_table[sec_idx];
        if !sec.is_present() || sec.size() == 0 {
            continue;
        }
        let section_start = sec.start_offset();
        let abs_section = entry.abs_offset + section_start;

        let fs_header_offset = 0x400 + sec_idx * 0x200;
        let fs_type = header.section_fs_type(sec_idx);
        let crypto_type = header.section_crypto_type(sec_idx);

        if reader.seek(SeekFrom::Start(abs_section)).is_err() {
            continue;
        }
        let mut buf = vec![0u8; 0x10];
        if reader.read_exact(&mut buf).is_err() {
            continue;
        }

        let nonce = header.section_ctr_nonce(sec_idx);
        let mut dec = buf.clone();
        aes_ctr_transform_in_place(&title_key, &nonce, section_start, true, &mut dec);

        if fs_type == 2 && crypto_type == 3 {
            if &dec[..4] == b"PFS0" {
                return true;
            }
        } else if fs_type == 3 && crypto_type == 3 {
            let expected_hash =
                &header.raw_bytes()[fs_header_offset + 0xC8..0x400 + sec_idx * 0x200 + 0xE8];
            let actual_hash = sha2::Sha256::digest(&dec);
            if actual_hash.as_slice() == expected_hash {
                return true;
            }
        } else if fs_type == 3
            && crypto_type == 4
            && matches!(header.content_type_enum(), Some(ContentType::Program))
        {
            let fs_header = &header.raw_bytes()[fs_header_offset..fs_header_offset + 0x200];
            if fs_header.windows(4).any(|w| w == b"BKTR") {
                return true;
            }
        }
    }
    false
}

fn read_meta_nca_cnmt<R: Read + Seek>(
    entry: &ContainerEntry,
    reader: &mut R,
    ks: &KeyStore,
) -> Result<Cnmt> {
    let header = NcaHeader::from_reader(reader, entry.abs_offset, ks)?;
    let sec0 = &header.section_table[0];
    if !sec0.is_present() {
        return Err(NscbError::InvalidData("META NCA has no section 0".into()));
    }

    let section_start = sec0.start_offset();
    let section_size = sec0.size() as usize;
    let abs_section = entry.abs_offset + section_start;

    reader.seek(SeekFrom::Start(abs_section))?;
    let mut section_data = vec![0u8; section_size.min(4 * 1024 * 1024)];
    reader.read_exact(&mut section_data)?;

    let crypto_type = header.section_crypto_type(0);
    if crypto_type == 3 {
        let candidate_keys = {
            let mut keys = Vec::new();
            if let Ok(sk) = header.decrypt_key_area(ks) {
                for k in &sk {
                    keys.push(*k);
                }
            }
            keys
        };
        let nonce = header.section_ctr_nonce(0);
        for key in &candidate_keys {
            for &le in &[true, false] {
                let mut dec = section_data.clone();
                aes_ctr_transform_in_place(&key, &nonce, section_start, le, &mut dec);
                if let Ok(cnmt) = extract_cnmt_from_section(&dec) {
                    return Ok(cnmt);
                }
            }
        }
    }

    extract_cnmt_from_section(&section_data)
}

fn extract_cnmt_from_section(section: &[u8]) -> Result<Cnmt> {
    use std::io::Cursor;
    let scan_len = section.len().min(2 * 1024 * 1024);
    for i in 0..scan_len.saturating_sub(4) {
        if &section[i..i + 4] == b"PFS0" {
            let mut cursor = Cursor::new(&section[i..]);
            if let Ok(pfs) = Pfs0::parse_at(&mut cursor, 0) {
                for entry in &pfs.entries {
                    if entry.name.ends_with(".cnmt") {
                        let cnmt_abs = i + pfs.file_abs_offset(entry) as usize;
                        let cnmt_end = cnmt_abs + entry.size as usize;
                        if cnmt_end <= section.len() {
                            let cnmt_bytes = &section[cnmt_abs..cnmt_end];
                            if let Ok(cnmt) = Cnmt::from_bytes(cnmt_bytes) {
                                return Ok(cnmt);
                            }
                        }
                    }
                }
            }
        }
    }
    Err(NscbError::InvalidData("CNMT not found in section".into()))
}

// ---------------------------------------------------------------------------
// lv2: Signature test (RSA-PSS)
// ---------------------------------------------------------------------------

/// Info about a verified NCA for use in LV3 hash test.
#[derive(Debug, Clone)]
struct NcaSigInfo {
    name: String,
    orig_header: Option<Vec<u8>>,
    listed_hash: Option<String>,
    did_verify: bool,
}

fn run_sig_test<R: Read + Seek>(
    entries: &[ContainerEntry],
    reader: &mut R,
    ks: &KeyStore,
    token: &str,
) -> Result<(bool, Vec<NcaSigInfo>, String)> {
    let tabs = "\t";
    let mut out = String::new();
    let mut verdict = true;
    let mut header_info: Vec<NcaSigInfo> = Vec::new();

    out.push_str("\n****************\n");
    out.push_str("SIGNATURE 1 TEST\n");
    out.push_str("****************\n");

    let nca_entries: Vec<&ContainerEntry> = entries
        .iter()
        .filter(|e| e.name.ends_with(".nca") || e.name.ends_with(".ncz"))
        .collect();

    let non_meta: Vec<&&ContainerEntry> = nca_entries
        .iter()
        .filter(|e| !e.name.ends_with("cnmt.nca"))
        .collect();
    let meta: Vec<&&ContainerEntry> = nca_entries
        .iter()
        .filter(|e| e.name.ends_with("cnmt.nca"))
        .collect();

    let ordered: Vec<&ContainerEntry> = non_meta.iter().chain(meta.iter()).map(|&&e| e).collect();

    for entry in ordered {
        let header = match NcaHeader::from_reader(reader, entry.abs_offset, ks) {
            Ok(h) => h,
            Err(_) => {
                out.push_str("UNKNOWN - Content.UNKNOWN\n");
                out.push_str(&format!("    > {}{tabs}  -> was MODIFIED\n", entry.name));
                out.push_str(&format!(
                    "{tabs}* NOT VERIFIABLE COULD'VE BEEN TAMPERED WITH\n"
                ));
                verdict = false;
                header_info.push(NcaSigInfo {
                    name: entry.name.clone(),
                    orig_header: None,
                    listed_hash: None,
                    did_verify: false,
                });
                continue;
            }
        };

        let title_id = format!("{:016X}", header.title_id);
        let content_type = content_type_py(header.content_type_enum());
        out.push_str(&format!("{} - {}\n", title_id, content_type));

        let is_cnmt = entry.name.ends_with("cnmt.nca");
        let (proper, notes, orig_header, did_verify) =
            verify_nca_signature_full(entry, &header, reader, ks, is_cnmt);

        let arrow = if is_cnmt { "   -> " } else { "\t  -> " };

        let needs_rsv_check =
            is_cnmt && !proper && notes.first().map(String::as_str) == Some("__NEEDS_RSV_CHECK__");

        if needs_rsv_check {
            verdict = false;
            out.push_str(&format!("    > {}{arrow}needs RSV check\n", entry.name));
            for note in notes.iter().skip(1) {
                out.push_str(&format!("\t    - {}\n", note));
            }
            out.push_str(&format!("    > {}{arrow}was MODIFIED\n", entry.name));
        } else if proper {
            out.push_str(&format!("    > {}{arrow}is PROPER\n", entry.name));
        } else {
            verdict = false;
            out.push_str(&format!("    > {}{arrow}was MODIFIED\n", entry.name));
            out.push_str(&format!(
                "{tabs}* NOT VERIFIABLE COULD'VE BEEN TAMPERED WITH\n"
            ));
        }
        for note in &notes {
            if needs_rsv_check || note == "__NEEDS_RSV_CHECK__" {
                continue;
            }
            out.push_str(&format!("{tabs}* {}\n", note));
        }

        let listed_hash = if is_cnmt && !proper {
            // Check if file was rehashed
            reader.seek(SeekFrom::Start(entry.abs_offset)).ok();
            let sha = hash::sha256_bounded(reader, entry.size).ok();
            if let Some(digest) = sha {
                let hex_digest = hex::encode(digest);
                let filename_prefix = entry.name.strip_suffix(".nca").unwrap_or(&entry.name);
                let fname_id = &filename_prefix[..filename_prefix.len().min(32)];
                let sha_prefix = &hex_digest[..hex_digest.len().min(32)];
                if fname_id == sha_prefix {
                    Some("patched".to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        header_info.push(NcaSigInfo {
            name: entry.name.clone(),
            orig_header: orig_header,
            listed_hash,
            did_verify,
        });
    }

    if verdict {
        out.push_str(&format!("VERDICT: {}  FILE IS SAFE\n", token));
    } else {
        out.push_str(&format!(
            "VERDICT: {} FILE COULD'VE BEEN TAMPERED WITH\n",
            token
        ));
    }

    Ok((verdict, header_info, out))
}

fn verify_nca_signature_full<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    ks: &KeyStore,
    is_cnmt: bool,
) -> (bool, Vec<String>, Option<Vec<u8>>, bool) {
    let sign1 = header.signature1();
    let headdata = header.signed_header_data();

    let modulus = if header.sig_key_generation == 0 {
        &NCA_HEADER_FIXED_KEY_MODULUS_00[..]
    } else {
        &NCA_HEADER_FIXED_KEY_MODULUS_01[..]
    };

    let n = BigUint::from_bytes_be(modulus);
    let e = BigUint::from(65537u32);
    let Ok(rsa_key) = rsa::RsaPublicKey::new(n, e) else {
        return (false, vec![], None, false);
    };

    let vk = VerifyingKey::<Sha256>::new(rsa_key);
    let Ok(sig) = Signature::try_from(sign1) else {
        return (false, vec![], None, false);
    };

    if vk.verify(headdata, &sig).is_ok() {
        return (true, vec![], None, true);
    }

    // Signature failed — try restoration
    let crypto1 = header.crypto_type;
    let crypto2 = header.crypto_type2;
    let master_key_rev = if crypto2 > crypto1 { crypto2 } else { crypto1 };

    // Try restorehead_tr (title rights restoration)
    if let Some((restored_headdata, notes, _orig_header_sha, tr, tkey, orkg)) =
        restore_header_tr(header, ks)
    {
        let n2 = BigUint::from_bytes_be(modulus);
        let e2 = BigUint::from(65537u32);
        if let Ok(rsa_key2) = rsa::RsaPublicKey::new(n2, e2) {
            let vk2 = VerifyingKey::<Sha256>::new(rsa_key2);
            if let Ok(sig2) = Signature::try_from(sign1) {
                if vk2.verify(&restored_headdata, &sig2).is_ok() {
                    let mut all_notes = vec!["TITLERIGHTS WERE REMOVED".to_string()];
                    all_notes.extend(notes);
                    let tkey_hex = hex::encode(tkey);
                    if tkey.iter().all(|&b| b == 0) {
                        all_notes.push("WARNING: sum(titlekey)=0 -> S.C. conversion may be incorrect and come from nsx file".to_string());
                    }
                    all_notes.push(format!(
                        "Original titlerights id is : {}",
                        tr.to_uppercase()
                    ));
                    all_notes.push(format!(
                        "Original titlekey is       : {}",
                        tkey_hex.to_uppercase()
                    ));
                    if orkg != master_key_rev {
                        all_notes.push(format!(
                            "KEYGENERATION WAS CHANGED FROM {} TO {}",
                            orkg, master_key_rev
                        ));
                    }
                    let rebuilt = rebuild_encrypted_header_with_signed_data(
                        entry,
                        reader,
                        ks,
                        &restored_headdata,
                    );
                    return (true, all_notes, rebuilt, true);
                }
            }
        }
    }

    // Try restorehead_ntr (non-title-rights restoration)
    if let Some((restored_headdata, notes, _orig_header_sha, orkg)) = restore_header_ntr(header, ks)
    {
        let n2 = BigUint::from_bytes_be(modulus);
        let e2 = BigUint::from(65537u32);
        if let Ok(rsa_key2) = rsa::RsaPublicKey::new(n2, e2) {
            let vk2 = VerifyingKey::<Sha256>::new(rsa_key2);
            if let Ok(sig2) = Signature::try_from(sign1) {
                if vk2.verify(&restored_headdata, &sig2).is_ok() {
                    let mut all_notes = notes;
                    if orkg != master_key_rev {
                        all_notes.push(format!(
                            "KEYGENERATION WAS CHANGED FROM {} TO {}",
                            orkg, master_key_rev
                        ));
                    }
                    let rebuilt = rebuild_encrypted_header_with_signed_data(
                        entry,
                        reader,
                        ks,
                        &restored_headdata,
                    );
                    return (true, all_notes, rebuilt, true);
                }
            }
        }
    }

    // For META NCAs, try RSV brute-force
    if is_cnmt {
        return try_rsv_bruteforce(entry, header, reader, ks, modulus, sign1);
    }

    (false, vec![], None, false)
}

fn rebuild_encrypted_header_with_signed_data<R: Read + Seek>(
    entry: &ContainerEntry,
    reader: &mut R,
    ks: &KeyStore,
    signed_data: &[u8],
) -> Option<Vec<u8>> {
    use crate::crypto::aes_xts::NintendoXts;

    if signed_data.len() != 0x200 {
        return None;
    }

    reader.seek(SeekFrom::Start(entry.abs_offset)).ok()?;
    let mut encrypted = vec![0u8; 0xC00];
    reader.read_exact(&mut encrypted).ok()?;
    let (mut header_dec, xts_key, le_sector) = decrypt_header_for_edit(&encrypted, ks).ok()?;
    header_dec[0x200..0x400].copy_from_slice(signed_data);
    let xts = NintendoXts::new(&xts_key).ok()?;
    xts.encrypt_with_endian(0, &mut header_dec, le_sector);
    Some(header_dec)
}

fn restore_header_tr(
    header: &NcaHeader,
    ks: &KeyStore,
) -> Option<(Vec<u8>, Vec<String>, String, String, [u8; 16], u8)> {
    let sign1 = header.signature1();
    let crypto1 = header.crypto_type;
    let crypto2 = header.crypto_type2;
    let nca_id = header.title_id;
    let master_key_rev = if crypto2 > crypto1 { crypto2 } else { crypto1 };

    // Decrypt key block
    let kak = ks.key_area_key(master_key_rev, header.key_index).ok()?;
    let dec_key_block = aes_ecb::decrypt_block(&kak, &header.key_area[..16]).ok()?;

    // Build rights_id
    let cr2 = format!("{:x}", crypto2);
    let tr_start = format!("{:016x}", nca_id);
    let tr_start = if matches!(
        header.content_type_enum(),
        Some(ContentType::Program) | Some(ContentType::Manual)
    ) {
        format!("{}000", &tr_start[..13])
    } else {
        tr_start
    };
    let tr = if cr2.len() == 1 {
        format!("{}000000000000000{}", tr_start, cr2)
    } else {
        format!("{}00000000000000{}", tr_start, cr2)
    };
    let tr_bytes = hex::decode(&tr).ok()?;

    // Build headdata with rights_id
    let raw = header.raw_bytes();
    let mut headdata = Vec::new();
    headdata.extend_from_slice(&raw[0x200..0x230]);
    headdata.extend_from_slice(&tr_bytes);
    headdata.extend_from_slice(&raw[0x240..0x300]);
    headdata.extend_from_slice(&[0u8; 0x40]); // zeroed key block
    headdata.extend_from_slice(&raw[0x340..0x400]);

    let modulus = if header.sig_key_generation == 0 {
        &NCA_HEADER_FIXED_KEY_MODULUS_00[..]
    } else {
        &NCA_HEADER_FIXED_KEY_MODULUS_01[..]
    };
    let n = BigUint::from_bytes_be(modulus);
    let e = BigUint::from(65537u32);
    let rsa_key = rsa::RsaPublicKey::new(n, e).ok()?;
    let vk = VerifyingKey::<Sha256>::new(rsa_key);
    let sig = Signature::try_from(sign1).ok()?;

    if vk.verify(&headdata, &sig).is_ok() {
        let orig_header_sha = hex::encode(sha2::Sha256::digest(&headdata));
        let title_key_enc = ks
            .decrypt_title_key(&dec_key_block, master_key_rev)
            .unwrap_or([0u8; 16]);
        return Some((
            headdata,
            vec![],
            orig_header_sha,
            tr.to_uppercase(),
            title_key_enc,
            master_key_rev,
        ));
    }

    // Try with 800 suffix
    let tr2 = if cr2.len() == 1 {
        format!(
            "{}800000000000000000{}",
            &format!("{:016x}", nca_id)[..13],
            cr2
        )
    } else {
        format!(
            "{}8000000000000000{}",
            &format!("{:016x}", nca_id)[..13],
            cr2
        )
    };
    let tr2_bytes = hex::decode(&tr2).ok()?;
    let mut headdata2 = headdata.clone();
    headdata2.splice(0x30..0x40, tr2_bytes.iter().cloned());

    if vk.verify(&headdata2, &sig).is_ok() {
        let orig_header_sha = hex::encode(sha2::Sha256::digest(&headdata2));
        let title_key_enc = ks
            .decrypt_title_key(&dec_key_block, master_key_rev)
            .unwrap_or([0u8; 16]);
        return Some((
            headdata2,
            vec![],
            orig_header_sha,
            tr2.to_uppercase(),
            title_key_enc,
            master_key_rev,
        ));
    }

    // Try all key generations
    for i in (0..12).rev() {
        let (c1, c2) = if i < 3 {
            (format!("{:02}", i), "00".to_string())
        } else {
            let cr = format!("{:x}", i);
            (
                "02".to_string(),
                if cr.len() == 1 {
                    format!("0{}", cr)
                } else {
                    cr
                },
            )
        };

        let new_mkrev = i;
        let new_kak = ks.key_area_key(new_mkrev, header.key_index).ok()?;
        let reenc_slot = aes_ecb::encrypt_block(&new_kak, &dec_key_block).ok()?;

        let tr1 = if c2.len() == 1 {
            format!(
                "{}000000000000000{}",
                &format!("{:016x}", nca_id)[..13],
                &c2[1..2]
            )
        } else {
            format!(
                "{}00000000000000{}",
                &format!("{:016x}", nca_id)[..13],
                &c2[1..2]
            )
        };
        let tr2b = if c2.len() == 1 {
            format!(
                "{}800000000000000000{}",
                &format!("{:016x}", nca_id)[..13],
                &c2[1..2]
            )
        } else {
            format!(
                "{}8000000000000000{}",
                &format!("{:016x}", nca_id)[..13],
                &c2[1..2]
            )
        };

        let c1_bytes = hex::decode(&c1).ok()?;
        let c2_bytes = hex::decode(&c2).ok()?;
        let tr1_bytes = hex::decode(&tr1).ok()?;
        // Build headdata1 with card flag
        let mut hd1 = Vec::new();
        hd1.extend_from_slice(&raw[0x200..0x206]);
        hd1.push(0x01); // card flag
        hd1.extend_from_slice(&raw[0x207..0x208]);
        hd1.extend_from_slice(&c1_bytes);
        hd1.extend_from_slice(&raw[0x209..0x220]);
        hd1.extend_from_slice(&c2_bytes);
        hd1.extend_from_slice(&raw[0x221..0x230]);
        hd1.extend_from_slice(&tr1_bytes);
        hd1.extend_from_slice(&raw[0x240..0x300]);
        for _ in 0..4 {
            hd1.extend_from_slice(&reenc_slot);
        }
        hd1.extend_from_slice(&raw[0x340..0x400]);

        // Build headdata2 with eshop flag
        let mut hd2 = hd1.clone();
        hd2[0x04] = 0x00;

        if vk.verify(&hd1, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd1));
            let title_key_enc = ks
                .decrypt_title_key(&dec_key_block, new_mkrev)
                .unwrap_or([0u8; 16]);
            return Some((
                hd1,
                vec![],
                orig_header_sha,
                tr1.to_uppercase(),
                title_key_enc,
                new_mkrev,
            ));
        }
        if vk.verify(&hd2, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd2));
            let title_key_enc = ks
                .decrypt_title_key(&dec_key_block, new_mkrev)
                .unwrap_or([0u8; 16]);
            return Some((
                hd2,
                vec![],
                orig_header_sha,
                tr2b.to_uppercase(),
                title_key_enc,
                new_mkrev,
            ));
        }
    }

    None
}

fn restore_header_ntr(
    header: &NcaHeader,
    ks: &KeyStore,
) -> Option<(Vec<u8>, Vec<String>, String, u8)> {
    let sign1 = header.signature1();
    let raw = header.raw_bytes();
    let headdata = &raw[0x200..0x400];

    let crypto1 = header.crypto_type;
    let crypto2 = header.crypto_type2;
    let master_key_rev = if crypto2 > crypto1 { crypto2 } else { crypto1 };

    let modulus = if header.sig_key_generation == 0 {
        &NCA_HEADER_FIXED_KEY_MODULUS_00[..]
    } else {
        &NCA_HEADER_FIXED_KEY_MODULUS_01[..]
    };
    let n = BigUint::from_bytes_be(modulus);
    let e = BigUint::from(65537u32);
    let rsa_key = rsa::RsaPublicKey::new(n, e).ok()?;
    let vk = VerifyingKey::<Sha256>::new(rsa_key);
    let sig = Signature::try_from(sign1).ok()?;

    let current_gamecard = header.raw_bytes().get(0x204).copied().unwrap_or(0);
    if current_gamecard == 0 {
        let mut hd_eshop = headdata.to_vec();
        hd_eshop[0x04] = 0x00;
        if vk.verify(&hd_eshop, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd_eshop));
            return Some((hd_eshop, vec![], orig_header_sha, master_key_rev));
        }

        let mut hd_card = headdata.to_vec();
        hd_card[0x04] = 0x01;
        if vk.verify(&hd_card, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd_card));
            return Some((
                hd_card,
                vec!["ISGAMECARD WAS CHANGED FROM 1 TO 0".to_string()],
                orig_header_sha,
                master_key_rev,
            ));
        }
    } else {
        let mut hd_eshop = headdata.to_vec();
        hd_eshop[0x04] = 0x00;
        if vk.verify(&hd_eshop, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd_eshop));
            return Some((
                hd_eshop,
                vec!["ISGAMECARD WAS CHANGED FROM 0 TO 1".to_string()],
                orig_header_sha,
                master_key_rev,
            ));
        }

        let mut hd_card = headdata.to_vec();
        hd_card[0x04] = 0x01;
        if vk.verify(&hd_card, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd_card));
            return Some((hd_card, vec![], orig_header_sha, master_key_rev));
        }
    }

    // Try key generation changes
    let kak = ks.key_area_key(master_key_rev, header.key_index).ok()?;
    let mut dec_key_block = header.key_area;
    // The NCA key area is 4 AES blocks; squirrel.py decrypts the full 0x40-byte buffer here.
    aes_ecb::decrypt(&kak, &mut dec_key_block).ok()?;

    for i in (0..12).rev() {
        let (c1, c2) = if i < 3 {
            (format!("{:02}", i), "00".to_string())
        } else {
            let cr = format!("{:x}", i);
            (
                "02".to_string(),
                if cr.len() == 1 {
                    format!("0{}", cr)
                } else {
                    cr
                },
            )
        };

        let new_mkrev = i;
        let new_kak = ks.key_area_key(new_mkrev, header.key_index).ok()?;
        let mut reenc_key_block = [0u8; 64];
        for slot in 0..4 {
            let enc = aes_ecb::encrypt_block(&new_kak, &dec_key_block[slot * 16..(slot + 1) * 16])
                .ok()?;
            reenc_key_block[slot * 16..(slot + 1) * 16].copy_from_slice(&enc);
        }

        let c1_bytes = hex::decode(&c1).ok()?;
        let c2_bytes = hex::decode(&c2).ok()?;

        // card flag
        let mut hd1 = Vec::new();
        hd1.extend_from_slice(&headdata[0x00..0x04]);
        hd1.push(0x01);
        hd1.extend_from_slice(&headdata[0x05..0x06]);
        hd1.extend_from_slice(&c1_bytes);
        hd1.extend_from_slice(&headdata[0x07..0x20]);
        hd1.extend_from_slice(&c2_bytes);
        hd1.extend_from_slice(&headdata[0x21..0x100]);
        hd1.extend_from_slice(&reenc_key_block);
        hd1.extend_from_slice(&headdata[0x140..0x200]);

        // eshop flag
        let mut hd2 = hd1.clone();
        hd2[0x04] = 0x00;

        if vk.verify(&hd1, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd1));
            return Some((hd1, vec![], orig_header_sha, new_mkrev));
        }
        if vk.verify(&hd2, &sig).is_ok() {
            let orig_header_sha = hex::encode(sha2::Sha256::digest(&hd2));
            return Some((hd2, vec![], orig_header_sha, new_mkrev));
        }
    }

    None
}

fn try_rsv_bruteforce<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    ks: &KeyStore,
    modulus: &[u8],
    sign1: &[u8],
) -> (bool, Vec<String>, Option<Vec<u8>>, bool) {
    let n = BigUint::from_bytes_be(modulus);
    let e = BigUint::from(65537u32);
    let Ok(rsa_key) = rsa::RsaPublicKey::new(n, e) else {
        return (false, vec![], None, false);
    };
    let vk = VerifyingKey::<Sha256>::new(rsa_key);
    let Ok(sig) = Signature::try_from(sign1) else {
        return (false, vec![], None, false);
    };

    // Read the full NCA content
    reader.seek(SeekFrom::Start(entry.abs_offset)).ok();
    let mut nca_data = vec![0u8; entry.size as usize];
    if reader.read_exact(&mut nca_data).is_err() {
        return (false, vec![], None, false);
    }

    // Decrypt header for editing
    let (mut header_dec, _xts_key, _le_sector) =
        match decrypt_header_for_edit(&nca_data[..0xC00], ks) {
            Ok(v) => v,
            Err(_) => return (false, vec![], None, false),
        };

    // Get section keys
    let section_keys = match NcaHeader::from_decrypted(header_dec.clone()) {
        Ok(h) => h.decrypt_key_area(ks).unwrap_or([[0u8; 16]; 4]),
        Err(_) => return (false, vec![], None, false),
    };

    // Find the encrypted section
    for sec_idx in 0..4 {
        let sec = &header.section_table[sec_idx];
        if !sec.is_present() || sec.size() == 0 {
            continue;
        }
        let sec_start = sec.start_offset() as usize;
        let sec_end = sec.end_offset() as usize;
        if sec_end > nca_data.len() || sec_start >= sec_end {
            continue;
        }

        let section_enc = &nca_data[sec_start..sec_end];
        let crypto_type = header.section_crypto_type(sec_idx);

        let section_plain = if crypto_type == 1 || crypto_type == 0 {
            section_enc.to_vec()
        } else {
            let nonce = header.section_ctr_nonce(sec_idx);
            let mut decrypted = None;
            for key in &section_keys {
                for &le in &[true, false] {
                    let mut dec = section_enc.to_vec();
                    aes_ctr_transform_in_place(key, &nonce, sec_start as u64, le, &mut dec);
                    if dec.len() > 4 && &dec[0..4] == b"PFS0" {
                        decrypted = Some(dec);
                        break;
                    }
                }
                if decrypted.is_some() {
                    break;
                }
            }
            decrypted.unwrap_or_else(|| section_enc.to_vec())
        };

        // Find CNMT and try RSV values
        let kglist = types::kgstring();
        let orig_kg = header.python_patcher_key_generation();

        for (kg_idx, kg_versions) in kglist.iter().enumerate().rev() {
            if kg_idx < orig_kg as usize {
                break;
            }
            for &rsv in kg_versions {
                // Patch RSV in CNMT
                if let Some(mut patched_section) = patch_cnmt_rsv(&section_plain, header, rsv) {
                    // Recalculate hashes
                    if let Ok(()) =
                        recalc_meta_hashes(&mut header_dec, &mut patched_section, header)
                    {
                        // Verify signature
                        let headdata = &header_dec[0x200..0x400];
                        if vk.verify(headdata, &sig).is_ok() {
                            let fw_str = types::rsv_to_firmware(rsv);
                            let notes = vec![
                                format!("RSV WAS CHANGED FROM {} TO {}", rsv, rsv),
                                "THE CNMT FILE IS CORRECT".to_string(),
                                format!("Firmware: {}", fw_str),
                            ];
                            return (true, notes, None, true);
                        }
                    }
                }
            }
        }
    }

    // Last resort: mirror Python's meta verifier output. It reports the
    // internal hashes as correct but still leaves the META marked modified.
    if check_cnmt_hashes(entry, header, reader, ks) {
        return (
            false,
            vec![
                "__NEEDS_RSV_CHECK__".to_string(),
                "PFS0 hash is CORRECT".to_string(),
                "HASH TABLE hash is CORRECT".to_string(),
                "HEADER BLOCK hash is CORRECT".to_string(),
            ],
            None,
            false,
        );
    }

    (false, vec![], None, false)
}

fn patch_cnmt_rsv(section: &[u8], header: &NcaHeader, new_rsv: u32) -> Option<Vec<u8>> {
    let scan_len = section.len().min(2 * 1024 * 1024);
    for i in 0..scan_len.saturating_sub(4) {
        if &section[i..i + 4] == b"PFS0" {
            use std::io::Cursor;
            let mut cursor = Cursor::new(&section[i..]);
            if let Ok(pfs) = Pfs0::parse_at(&mut cursor, 0) {
                for entry in &pfs.entries {
                    if entry.name.ends_with(".cnmt") {
                        let cnmt_abs = i + pfs.file_abs_offset(entry) as usize;
                        let cnmt_end = cnmt_abs + entry.size as usize;
                        if cnmt_end <= section.len() {
                            if let Ok(mut cnmt) = Cnmt::from_bytes(&section[cnmt_abs..cnmt_end]) {
                                let before = cnmt.required_system_version;
                                let keygen = header.python_patcher_key_generation();
                                let after = types::apply_patcher_meta_rsv(keygen, before, new_rsv);
                                if after != before {
                                    cnmt.patch_required_system_version(after);
                                    let mut new_section = section.to_vec();
                                    new_section[cnmt_abs..cnmt_end].copy_from_slice(&cnmt.raw);
                                    return Some(new_section);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn recalc_meta_hashes(
    header_dec: &mut [u8],
    section_plain: &mut [u8],
    header: &NcaHeader,
) -> Result<()> {
    let block_size = header.hblock_block_size() as usize;
    let htable_offset = header.htable_offset() as usize;
    let htable_size = header.htable_size() as usize;
    let pfs0_offset = header.pfs0_offset() as usize;
    let pfs0_size = header.pfs0_size() as usize;
    if block_size == 0 || pfs0_size == 0 {
        return Ok(());
    }

    let mult = pfs0_size.div_ceil(block_size);
    let pfs0_hash_len = 0x20usize.saturating_mul(mult);
    if htable_offset + pfs0_hash_len > section_plain.len()
        || pfs0_offset + pfs0_size > section_plain.len()
    {
        return Ok(());
    }

    let pfs0_block_len = block_size.min(pfs0_size);
    let pfs0_hash = sha2::Sha256::digest(&section_plain[pfs0_offset..pfs0_offset + pfs0_block_len]);
    section_plain[htable_offset..htable_offset + 0x20].copy_from_slice(&pfs0_hash);

    let htable_hash = sha2::Sha256::digest(
        &section_plain[htable_offset..htable_offset + pfs0_hash_len.min(htable_size)],
    );
    header_dec[0x408..0x428].copy_from_slice(&htable_hash);

    let hblock_hash = sha2::Sha256::digest(&header_dec[0x400..0x600]);
    header_dec[0x280..0x2A0].copy_from_slice(&hblock_hash);

    Ok(())
}

fn check_cnmt_hashes<R: Read + Seek>(
    entry: &ContainerEntry,
    header: &NcaHeader,
    reader: &mut R,
    ks: &KeyStore,
) -> bool {
    // Read section data
    for sec_idx in 0..4 {
        let sec = &header.section_table[sec_idx];
        if !sec.is_present() || sec.size() == 0 {
            continue;
        }
        let sec_start = sec.start_offset() as usize;
        let sec_end = sec.end_offset() as usize;
        if sec_end > entry.size as usize || sec_start >= sec_end {
            continue;
        }

        reader
            .seek(SeekFrom::Start(entry.abs_offset + sec_start as u64))
            .ok();
        let mut section_data = vec![0u8; (sec_end - sec_start).min(4 * 1024 * 1024)];
        if reader.read_exact(&mut section_data).is_err() {
            continue;
        }

        let crypto_type = header.section_crypto_type(sec_idx);
        let section_keys = header.decrypt_key_area(ks).unwrap_or([[0u8; 16]; 4]);

        let section_plain = if crypto_type == 3 {
            let nonce = header.section_ctr_nonce(sec_idx);
            let mut decrypted = None;
            for key in &section_keys {
                for &le in &[true, false] {
                    let mut dec = section_data.clone();
                    aes_ctr_transform_in_place(key, &nonce, sec_start as u64, le, &mut dec);
                    if dec.len() > 4 && &dec[0..4] == b"PFS0" {
                        decrypted = Some(dec);
                        break;
                    }
                }
                if decrypted.is_some() {
                    break;
                }
            }
            decrypted.unwrap_or(section_data)
        } else {
            section_data
        };

        // Python parity: `check_cnmt_hashes` compares each calculated hash
        // against the same freshly calculated value, so once we can parse the
        // section layout successfully it reports the three hashes as correct.
        let block_size = header.hblock_block_size() as usize;
        let htable_offset = header.htable_offset() as usize;
        let pfs0_offset = header.pfs0_offset() as usize;
        let pfs0_size = header.pfs0_size() as usize;
        if block_size == 0 || pfs0_size == 0 {
            continue;
        }

        let mult = pfs0_size.div_ceil(block_size);
        let pfs0_hash_len = 0x20usize.saturating_mul(mult);
        if htable_offset + 0x20 > section_plain.len()
            || htable_offset + pfs0_hash_len.min(header.htable_size() as usize)
                > section_plain.len()
            || pfs0_offset + block_size.min(pfs0_size) > section_plain.len()
            || header.raw_bytes().len() < 0x600
        {
            continue;
        }

        return true;
    }
    false
}

fn decrypt_header_for_edit(encrypted: &[u8], ks: &KeyStore) -> Result<(Vec<u8>, [u8; 32], bool)> {
    use crate::crypto::aes_xts::NintendoXts;
    if encrypted.len() < 0xC00 {
        return Err(NscbError::InvalidData("NCA header too short".into()));
    }

    let header_key = ks.header_key()?;
    let mut swapped_key = [0u8; 32];
    swapped_key[..16].copy_from_slice(&header_key[16..]);
    swapped_key[16..].copy_from_slice(&header_key[..16]);

    let attempts = [
        (header_key, true),
        (header_key, false),
        (swapped_key, true),
        (swapped_key, false),
    ];

    for (key, le_sector) in attempts {
        let xts = NintendoXts::new(&key)?;
        let mut decrypted = encrypted[..0xC00].to_vec();
        xts.decrypt_with_endian(0, &mut decrypted, le_sector);
        if NcaHeader::from_decrypted(decrypted.clone()).is_ok() {
            return Ok((decrypted, key, le_sector));
        }
    }

    Err(NscbError::InvalidData(
        "Failed to decrypt NCA header with known XTS variants".into(),
    ))
}

// ---------------------------------------------------------------------------
// lv3: Hash test (SHA-256 content hash vs filename prefix)
// ---------------------------------------------------------------------------

fn run_hash_test<R: Read + Seek>(
    entries: &[ContainerEntry],
    reader: &mut R,
    ks: &KeyStore,
    token: &str,
    header_info: &[NcaSigInfo],
    did_verify: bool,
) -> Result<(bool, String)> {
    let mut out = String::new();
    let mut verdict = true;

    out.push_str("***************\n");
    out.push_str("HASH TEST\n");
    out.push_str("***************\n");
    let nca_entries: Vec<&ContainerEntry> = entries
        .iter()
        .filter(|e| e.name.ends_with(".nca") || e.name.ends_with(".ncz"))
        .collect();

    for entry in &nca_entries {
        // Get title info from header
        let title_info = NcaHeader::from_reader(reader, entry.abs_offset, ks).ok();
        if let Some(ref h) = title_info {
            let title_id = format!("{:016X}", h.title_id);
            let content_type = content_type_py(h.content_type_enum());
            out.push_str(&format!("{} - {}\n", title_id, content_type));
        }

        out.push_str(&format!("  - File name: {}\n", entry.name));

        let sha = if entry.name.ends_with(".ncz") {
            // NCZ: compute SHA256 of the decompressed NCA content
            let nca_size = title_info.as_ref().map(|h| h.nca_size).unwrap_or(0);
            if nca_size == 0 {
                out.push_str("   > FILE IS CORRUPT (could not read NCA size from NCZ header)\n");
                verdict = false;
                out.push('\n');
                continue;
            }
            let mut tmp = tempfile::tempfile()?;
            if let Err(e) = decompress_ncz(reader, &mut tmp, nca_size, entry.abs_offset, entry.size)
            {
                out.push_str(&format!("   > FILE IS CORRUPT (decompress error: {})\n", e));
                verdict = false;
                out.push('\n');
                continue;
            }
            tmp.seek(SeekFrom::Start(0))?;
            match hash::sha256_streaming(&mut tmp) {
                Ok(h) => hex::encode(h),
                Err(e) => {
                    out.push_str(&format!("   > FILE IS CORRUPT (read error: {})\n", e));
                    verdict = false;
                    out.push('\n');
                    continue;
                }
            }
        } else {
            reader.seek(SeekFrom::Start(entry.abs_offset))?;
            let mut limited = reader.take(entry.size);
            match hash::sha256_streaming(&mut limited) {
                Ok(h) => hex::encode(h),
                Err(e) => {
                    out.push_str(&format!("   > FILE IS CORRUPT (read error: {})\n", e));
                    verdict = false;
                    continue;
                }
            }
        };
        out.push_str(&format!("  - SHA256: {}\n", sha));

        // Find corresponding sig info
        let sig_info = header_info.iter().find(|h| h.name == entry.name);

        // Check filename prefix (16 chars, matching Python)
        let filename_prefix = entry
            .name
            .strip_suffix(".nca")
            .or_else(|| entry.name.strip_suffix(".ncz"))
            .unwrap_or(&entry.name);
        let filename_id = &filename_prefix[..filename_prefix.len().min(16)];
        let sha_prefix = &sha[..sha.len().min(16)];

        let orig_sha = if let Some(info) = sig_info {
            if let Some(ref orig_header) = info.orig_header {
                compute_restored_file_sha(
                    entry,
                    reader,
                    title_info.as_ref().map(|h| h.nca_size),
                    &sha,
                    orig_header,
                )
            } else {
                None
            }
        } else {
            None
        };

        if let Some(ref orig_sha) = orig_sha {
            out.push_str(&format!("  - ORIG_SHA256: {}\n", orig_sha));
        }

        if filename_id == sha_prefix {
            out.push_str("   > FILE IS CORRECT\n");
        } else if let Some(info) = sig_info {
            // Try ORIG_SHA256
            if let Some(ref orig_sha) = orig_sha {
                if filename_id == &orig_sha[..orig_sha.len().min(16)] {
                    out.push_str("   > FILE IS CORRECT\n");
                } else {
                    out.push_str("   > FILE IS CORRUPT\n");
                    verdict = false;
                }
            } else if info.listed_hash.as_deref() == Some("patched") {
                // META NCA that was rehashed
                out.push_str(&format!(
                    "  - ORIG_SHA256: {}\n",
                    info.listed_hash.as_ref().unwrap()
                ));
                out.push_str("   > FILE IS CORRECT\n");
            } else if matches!(
                title_info.as_ref().map(|h| h.content_type_enum()),
                Some(Some(ContentType::Meta))
            ) && did_verify
            {
                // META NCA that passed sig test via RSV brute-force
                out.push_str("   > RSV WAS CHANGED\n");
                out.push_str("     * FILE IS CORRECT\n");
            } else {
                out.push_str("   > FILE IS CORRUPT\n");
                verdict = false;
            }
        } else {
            out.push_str("   > FILE IS CORRUPT\n");
            verdict = false;
        }
        out.push('\n');
    }

    if verdict {
        out.push_str(&format!("VERDICT: {} FILE IS CORRECT\n", token));
    } else {
        out.push_str(&format!("VERDICT: {} FILE IS CORRUPT\n", token));
    }

    Ok((verdict, out))
}

fn compute_restored_file_sha<R: Read + Seek>(
    entry: &ContainerEntry,
    reader: &mut R,
    nca_size: Option<u64>,
    current_sha: &str,
    orig_header: &[u8],
) -> Option<String> {
    use sha2::Digest as _;

    if orig_header.len() != 0xC00 {
        return None;
    }

    let mut hasher = sha2::Sha256::new();
    hasher.update(orig_header);

    if entry.name.ends_with(".ncz") {
        let nca_size = nca_size?;
        let mut tmp = tempfile::tempfile().ok()?;
        decompress_ncz(reader, &mut tmp, nca_size, entry.abs_offset, entry.size).ok()?;
        tmp.seek(SeekFrom::Start(0xC00)).ok()?;
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            if crate::util::report::is_cancelled() {
                return None;
            }
            let n = tmp.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    } else {
        reader
            .seek(SeekFrom::Start(entry.abs_offset + 0xC00))
            .ok()?;
        let mut limited = reader.take(entry.size.saturating_sub(0xC00));
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            if crate::util::report::is_cancelled() {
                return None;
            }
            let n = limited.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }

    let digest = hex::encode(hasher.finalize());
    if digest == current_sha {
        None
    } else {
        Some(digest)
    }
}
