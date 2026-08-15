use serde_json::{Map, Value, json};
use std::env;
use std::error::Error;
use std::process::Command;
use std::str::FromStr;

#[derive(Clone, Debug)]
struct ToolParameter {
    pub name: &'static str,
    pub ptype: &'static str, // string, number or boolean
    pub description: Option<&'static str>,
    pub is_required: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ToolDefinition {
    // tool_type: String, // Function
    pub name: &'static str,
    pub description: Option<&'static str>,
    pub parameters: &'static [ToolParameter],
    // callback:
}

impl ToolParameter {
    pub const fn string(name: &'static str) -> Self {
        Self {
            name,
            ptype: "string",
            description: None,
            is_required: true,
        }
    }

    pub const fn number(name: &'static str) -> Self {
        Self {
            name,
            ptype: "number",
            description: None,
            is_required: true,
        }
    }

    pub const fn set_required(mut self, is_required: bool) -> Self {
        self.is_required = is_required;
        self
    }

    pub const fn add_description(mut self, desc: &'static str) -> Self {
        self.description = Some(desc);
        self
    }
}

impl ToolDefinition {
    pub fn to_json(&self) -> Value {
        let mut tool = Map::new();
        tool.insert("name".into(), json!(self.name));
        if let Some(desc) = self.description.as_ref() {
            tool.insert("description".into(), json!(desc));
        }
        tool.insert(
            "parameters".into(),
            json!({
                "type": "object",
                "properties": Value::Object(Map::from_iter(self.parameters.iter().map(
                    |p| (p.name.into(),
                        if let Some(desc) = p.description {
                            json!({"type": p.ptype, "description": desc})
                        } else { json!({"type": p.ptype}) },
                    ),
                ))),
                "required": Value::Array(
                    self.parameters.iter().filter_map(|p| {
                        p.is_required.then(|| Value::String(p.name.into()))
                    }).collect(),
                )
            }),
        );
        json!({
            "type": "function",
            "function": Value::Object(tool)
        })
    }
}

pub const TOOL_DEFINITIONS: &'static [ToolDefinition] = &[
    ToolDefinition {
        name: "Shell",
        description: Some(
            "Run a shell/terminal command. Shell state is not preserved between calls of this tool. So, you may need to prepend any required directory changes or virtualenv activations at the beginning of each command with a &&",
        ),
        parameters: &[ToolParameter {
            ptype: "string",
            name: "command",
            description: Some("Command to run in the default OS shell."),
            is_required: true,
        }],
    },
    // ToolDefinition {
    //     name: "Find",
    //     description: Some(
    //         "Find file contents (for text/code files only). Will return file and approximate line location.",
    //     ),
    //     parameters: &[
    //         ToolParameter::string("extension"),
    //         ToolParameter::number("max_depth")
    //             .add_description("Maximum directory recursion depth from here, recommended = 4"),
    //         ToolParameter::number("max_files")
    //             .add_description("Maximum number of files to check, recommended = 1000"),
    //         ToolParameter::string("search_string").add_description(
    //             "Content to search for (either a semantic description or an exact snippet)",
    //         ),
    //     ],
    // },
    ToolDefinition {
        name: "FindInFile",
        description: Some(
            "Find file contents (for text/code files). Will return snippet and line number. Search is fuzzy and whitespace agnostic.",
        ),
        parameters: &[
            ToolParameter::string("file"),
            ToolParameter::string("search_string").add_description(
                "Content to search for (either a semantic description or an exact snippet)",
            ),
        ],
    },
    ToolDefinition {
        name: "ReadLines",
        description: Some("Read file contents from line until line (for text/code files)."),
        parameters: &[
            ToolParameter::string("file"),
            ToolParameter::number("line_from"),
            ToolParameter::number("line_to"),
        ],
    },
    ToolDefinition {
        name: "EditFile",
        description: Some(
            "Replace from `line_from`, up to and including `line_to` with `content`.",
        ),
        parameters: &[
            ToolParameter::string("file"),
            ToolParameter::number("line_from"),
            ToolParameter::number("line_to"),
            ToolParameter::string("content"),
        ],
    },
];

pub const TOOL_FUNCTIONS: &'static [fn(&str) -> Result<String, Box<dyn Error>>] =
    &[shell_tool, find_in_file, read_lines, edit_file];

pub fn call_tool(
    tool_name: &str,
    tool_argstring: &str,
    definitions: &[ToolDefinition],
    tool_funcs: &[fn(&str) -> Result<String, Box<dyn Error>>],
) -> String {
    if let Some(tool_index) = definitions.iter().position(|t| t.name == tool_name) {
        println!("calling {tool_name}\nwith args {tool_argstring}");
        let result = tool_funcs[tool_index](tool_argstring);
        result.unwrap_or_else(|err| format!("Tool error: {err}"))
    } else {
        format!("Tool \"{tool_name}\" not found!")
    }
}

fn get_string<'a>(json: &'a Value, argname: &str) -> Result<&'a str, String> {
    Ok(json
        .get(argname)
        .ok_or(format!("Missing {argname} argument"))?
        .as_str()
        .ok_or(format!("{argname} argument is not string"))?)
}

fn get_i64(json: &Value, argname: &str) -> Result<i64, String> {
    Ok(json
        .get(argname)
        .ok_or(format!("Missing {argname} argument"))?
        .as_i64()
        .ok_or(format!("{argname} argument is not string"))?)
}

pub fn shell_tool(args: &str) -> Result<String, Box<dyn Error>> {
    fn run_shell_command(cmd: &str) -> String {
        let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let output = Command::new(shell).arg("-c").arg(cmd).output().unwrap();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return format!("command failed: {}", stderr);
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    }
    // TODO: manage very long outputs somehow?
    Ok(serde_json::from_str::<Value>(&args)
        .map_err(|_| "Couldn't parse JSON")?
        .get("command")
        .ok_or("Couldn't parse JSON")?
        .as_str()
        .ok_or("Command argument is not string")
        .map(run_shell_command)?)
}

//TODO: make it fuzzier allow case insensitiveity maybe
pub fn find_in_file(args: &str) -> Result<String, Box<dyn Error>> {
    let parsed = serde_json::from_str::<Value>(&args).map_err(|_| "Couldn't parse JSON")?;
    let path = get_string(&parsed, "file")?;
    let search_string = get_string(&parsed, "search_string")?;

    const MAX_FILE_SIZE: u64 = 1_000_000;
    let metadata = std::fs::metadata(path).map_err(|e| format!("File error: {:?}", e))?;
    if metadata.len() > MAX_FILE_SIZE {
        Err(format!(
            "File too big: {} bytes, limit is {}",
            metadata.len(),
            MAX_FILE_SIZE
        )
        .into())
    } else {
        /* Remove all whitespace */
        let data = std::fs::read(path).map_err(|_| "Couldn't read file")?;
        let mut line_numbers = vec![0u32; data.len()];

        /* Label each characters's line number */
        let mut line_count = 0;
        for i in 0..data.len() {
            line_numbers[i] = line_count;
            if data[i] == b'\n' {
                line_count += 1;
            }
        }

        /* Strip whitespace and newlines (ascii/u8 only) */
        let mut data_stripped = Vec::with_capacity(data.len());
        let mut ln_stripped = Vec::with_capacity(data.len());
        let mut idxs_stripped = Vec::with_capacity(data.len());
        for i in 0..data.len() {
            if !(data[i] as char).is_ascii_whitespace() {
                data_stripped.push(data[i]);
                ln_stripped.push(line_numbers[i]);
                idxs_stripped.push(i as u32);
            }
        }

        /* Strip the search string */
        let mut ss_stripped = Vec::with_capacity(search_string.len());
        for i in 0..search_string.as_bytes().len() {
            if !(search_string.as_bytes()[i] as char).is_ascii_whitespace() {
                ss_stripped.push(search_string.as_bytes()[i]);
            }
        }

        /* Search for it in data. TODO: verify which occurences are exact in terms of whitespace maybe? */
        let occurences = data_stripped
            .windows(ss_stripped.len())
            .enumerate()
            .filter_map(|(i, w)| (w == ss_stripped).then_some(i))
            .collect::<Vec<_>>();

        let mut answer = String::new();

        if occurences.len() == 0 {
            answer = format!("No occurences of search string in file {}", path);
        } else {
            for pos in occurences {
                answer.push_str(&format!(
                    "Found an occurence on lines {}-{}:\n",
                    ln_stripped[pos],
                    ln_stripped[pos + ss_stripped.len() - 1]
                ));
                /* Indexes of chunk in original data */
                let (start_idx, end_idx) = (
                    idxs_stripped[pos] as usize,
                    idxs_stripped[pos + ss_stripped.len()] as usize,
                );
                answer.push_str(&format!(
                    "```\n{:?}\n```",
                    str::from_utf8(&data[start_idx..end_idx])
                ))
            }
        }

        println!("{}", answer);

        Ok(String::new())
    }
}

//TODO: make it fuzzier allow case insensitiveity maybe
pub fn read_lines(args: &str) -> Result<String, Box<dyn Error>> {
    let parsed = serde_json::from_str::<Value>(&args).map_err(|_| "Couldn't parse JSON")?;
    let path = get_string(&parsed, "file")?;
    let line_from = get_i64(&parsed, "line_from")? as usize;
    let line_to = get_i64(&parsed, "line_to")? as usize;

    let reader = std::io::BufReader::new(std::fs::File::open(path)?);
    use std::io::BufRead;
    let mut lines = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        if i >= line_from
            && i <= line_to
            && let Ok(line) = line
        {
            lines.push(line)
        }
    }

    Ok(lines.join("\n"))
}

pub fn edit_file(args: &str) -> Result<String, Box<dyn Error>> {
    let parsed = serde_json::from_str::<Value>(&args).map_err(|_| "Couldn't parse JSON")?;
    let path = get_string(&parsed, "file")?;
    let line_from = get_i64(&parsed, "line_from")? as usize;
    let line_to = get_i64(&parsed, "line_to")? as usize;
    let content = get_string(&parsed, "content")?;

    let reader = std::io::BufReader::new(std::fs::File::open(path)?);
    use std::io::BufRead;
    let mut lines = Vec::new();
    let mut inserted = false;
    for (i, line) in reader.lines().enumerate() {
        if !(i >= line_from && i <= line_to) {
            if let Ok(line) = line {
                lines.push(line)
            }
        } else if !inserted {
            lines.push(content.into());
        }
    }

    std::fs::write(path, lines.join("\n"))?;
    Ok("Edited file".into())
}
