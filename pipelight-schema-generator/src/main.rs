use std::fs;
use std::fs::File;
use std::io::{self, BufRead, stdin, Write};
use std::path::{Path, PathBuf};
use std::env;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use path_slash::PathExt;
use clap::Parser;

#[derive(Debug, Serialize, Deserialize)]
struct FilePath {
    path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PathComponents {
    full_path: String,
    filename: Option<String>,
    extension: Option<String>,
    parent_dir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JobData {
    #[serde(rename = "@context")]
    context: String,
    #[serde(rename = "@type")]
    type_name: String,
    files: Vec<FilePath>,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Optional path to a file containing a list of file paths, one per line.
    /// If not provided, input will be read from stdin.
    #[arg(short, long)]
    input_file: Option<String>,

    /// Name of the project for Buildbot configuration.
    #[arg(long, default_value = "pipelight-project")]
    project_name: String,

    /// Name of the Buildbot worker.
    #[arg(long, default_value = "nix-buildbot-worker")]
    worker_name: String,
}

fn generate_buildbot_nix_config(job_data: &JobData, project_name: &str, worker_name: &str) -> String {
    let mut nix_config = String::new();
    nix_config.push_str("{ config, pkgs, lib, ... }:\n\n");
    nix_config.push_str("{\n");
    nix_config.push_str(&format!("  # This NixOS module integrates {} job data with Buildbot.\n", project_name));
    nix_config.push_str("  # It defines a Buildbot factory and steps based on the analyzed file paths.\n");
    nix_config.push_str("  services.buildbot-master.settings = {\n");
    nix_config.push_str("    # Define a factory for pipelight jobs\n");
    nix_config.push_str(&format!("    factories.{}JobsFactory = {{\n", project_name.replace("-", "")));
    nix_config.push_str("      # Each step represents processing a file path\n");
    nix_config.push_str("      steps = [\n");

    for file_path in &job_data.files {
        nix_config.push_str(&format!(
            "        pkgs.buildbot.python3Packages.buildbot.steps.shell.Shell(name = \"process-{}-{}\", command = [\"echo\", \"Processing {}\"])\n",
            project_name.replace("-", "_"),
            Path::new(&file_path.path).file_name().unwrap_or_default().to_str().unwrap_or_default().replace("-", "_").replace(".", "_"),
            file_path.path
        ));
    }

    nix_config.push_str("      ];\n");
    nix_config.push_str("    };
");
    nix_config.push_str(&format!("    # Define a builder that uses the {} jobs factory\n", project_name));
    nix_config.push_str(&format!("    builders.{1}JobBuilder = {{ \n", project_name, project_name.replace("-", "")));
    nix_config.push_str(&format!("      workername = \"{}\";\n", worker_name));
    nix_config.push_str(&format!("      factory = \"{}JobsFactory\";\n", project_name.replace("-", "")));
    nix_config.push_str("    };
");
    nix_config.push_str("  };
");
    nix_config.push_str("}\n");
    nix_config
}

fn write_output_file(dir: &Path, filename: &str, content: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let file_path = dir.join(filename);
    let mut file = File::create(&file_path)?;
    file.write_all(content)?;
    Ok(file_path)
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    let current_dir = env::current_dir()?;
    println!("Current working directory: {:?}", current_dir);

    let reader: Box<dyn BufRead> = match cli.input_file {
        Some(file_name) => {
            let input_path = Path::new(&file_name);
            let display = input_path.display();
            let file = File::open(&input_path)
                .unwrap_or_else(|why| panic!("couldn't open {}: {}", display, why));
            Box::new(io::BufReader::new(file))
        },
        None => {
            println!("Reading input from stdin. Press Ctrl+D to finish.");
            Box::new(io::BufReader::new(stdin()))
        }
    };
    
    let mut files: Vec<FilePath> = Vec::new();
    let mut extensions: HashMap<String, usize> = HashMap::new();
    let mut filenames: HashMap<String, usize> = HashMap::new();
    let mut parent_dirs: HashMap<String, usize> = HashMap::new();

    for line in reader.lines() {
        match line {
            Ok(l) => {
                let trimmed_line = l.trim();
                if !trimmed_line.is_empty() {
                    files.push(FilePath { path: trimmed_line.to_string() });

                    let path_buf = PathBuf::from(trimmed_line);

                    if let Some(ext) = path_buf.extension().and_then(|s| s.to_str()) {
                        *extensions.entry(ext.to_string()).or_insert(0) += 1;
                    }
                    if let Some(filename) = path_buf.file_name().and_then(|s| s.to_str()) {
                        *filenames.entry(filename.to_string()).or_insert(0) += 1;
                    }
                    if let Some(parent) = path_buf.parent().and_then(|p| p.to_slash().map(|s| s.to_string())) {
                        *parent_dirs.entry(parent).or_insert(0) += 1;
                    }
                }
            },
            Err(e) => eprintln!("Error reading line: {}", e),
        }
    }

    let job_data = JobData {
        context: "https://schema.org".to_string(),
        type_name: "JobPosting".to_string(),
        files,
    };

    let output_dir = Path::new("./output");
    
    println!("\n--- Pipelight Schema Generator Summary ---");

    // --- Project Summary ---
    println!("\nProject Analysis:");
    println!("  Total unique files: {}", job_data.files.len());
    
    let mut sorted_parent_dirs: Vec<(&String, &usize)> = parent_dirs.iter().collect();
    sorted_parent_dirs.sort_by(|a, b| b.1.cmp(a.1));
    println!("  Top 5 most common parent directories:");
    for (dir, count) in sorted_parent_dirs.iter().take(5) {
        println!("    - {}: {} occurrences", dir, count);
    }
    println!("  Total unique parent directories: {}", parent_dirs.len());

    // --- Schema Generation Status ---
    println!("\nSchema & Config Generation Status:");
    
    // JSON-LD
    let json_ld_output = serde_json::to_string_pretty(&job_data)?;
    let json_ld_file = write_output_file(output_dir, "pipelight_job_schema.jsonld", json_ld_output.as_bytes())?;
    println!("  - JSON-LD Schema: Written to {}", json_ld_file.display());

    // CBOR
    let cbor_output = serde_cbor::to_vec(&job_data)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("CBOR serialization error: {}", e)))?;
    let cbor_file = write_output_file(output_dir, "pipelight_job_data.cbor", &cbor_output)?;
    println!("  - CBOR Data: Written to {}", cbor_file.display());
    let cbor_hex_file = write_output_file(output_dir, "pipelight_job_data.cbor.hex", hex::encode(&cbor_output).as_bytes())?;
    println!("  - CBOR Data (Hex): Written to {}", cbor_hex_file.display());

    // DASL
    let dasl_schema = r#"\
type FilePath {
  path: String
}

type JobData {
  "@context": String,
  "@type": String,
  files: Array<FilePath>
}
"#;
    let dasl_file = write_output_file(output_dir, "pipelight_job_schema.dasl", dasl_schema.as_bytes())?;
    println!("  - CBOR-DASL Schema Definition: Written to {}", dasl_file.display());

    // RDFa
    let mut rdfa_html = String::new();
    rdfa_html.push_str(r#"<div vocab=\"https://schema.org/\" typeof=\"JobPosting\">"#);
    rdfa_html.push_str("\n  <p>File paths for Pipelight jobs:</p>");
    rdfa_html.push_str("\n  <ul>");
    for file_path in &job_data.files {
        rdfa_html.push_str(&format!(
            "    <li property=\"files\" typeof=\"FilePath\">\n      <span property=\"path\">{}</span>\n    </li>",
            file_path.path
        ));
    }
    rdfa_html.push_str("\n  </ul>");
    rdfa_html.push_str("\n</div>");
    let rdfa_file = write_output_file(output_dir, "pipelight_job_schema.html", rdfa_html.as_bytes())?;
    println!("  - RDFa HTML Snippet: Written to {}", rdfa_file.display());

    // Buildbot Nix Config
    let buildbot_nix_config = generate_buildbot_nix_config(&job_data, &cli.project_name, &cli.worker_name);
    let buildbot_nix_file = write_output_file(output_dir, &format!("{}_buildbot_config.nix", cli.project_name.replace("-", "_")), buildbot_nix_config.as_bytes())?;
    println!("  - Buildbot Nix Configuration: Written to {}", buildbot_nix_file.display());

    Ok(())
}