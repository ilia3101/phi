use colored::Colorize;
use serde::{self, Deserialize, Serialize};
use serde_json::{self, Map, Value, json};
use std::io::{Read, Write};
use std::{collections::HashMap, time::Duration};

use std::env;
use std::process::Command;

fn run_shell_command(cmd: &str) -> String {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let output = Command::new(shell).arg("-c").arg(cmd).output().unwrap();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return format!("command failed: {}", stderr);
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

/* Chat-completions based agent :/ I really dislike the completions API.
 * This agent supports only function type tool calls.
 */

#[derive(Clone, Debug)]
struct Config {
    url: String,
    model_name: String,
    api_key: String,
    stream: bool,
    preserve_thinking: bool,                              // TODO
    truncate_long_tool_results_after_turn_finished: bool, // TODO: use this
}

/******************** Conversation history *********************/

#[derive(Clone, Debug, Default)]
struct Message {
    role: String,
    content: String,
    reasoning_content: Option<String>,
    tool_call_id: Option<String>,
    tool_calls: Vec<ToolCall>,
}

impl Message {
    pub fn to_json(&self) -> Value {
        let mut message = Map::new();
        message.insert("role".into(), json!(self.role));
        message.insert("content".into(), json!(self.content));
        self.reasoning_content
            .as_ref()
            .map(|r| message.insert("reasoning_content".into(), json!(r)));
        if self.tool_calls.len() > 0 {
            message.insert(
                "tool_calls".into(),
                self.tool_calls.iter().map(ToolCall::to_json).collect(),
            );
        }
        Value::Object(message)
    }

    pub fn empty() -> Self {
        Self {
            role: String::new(),
            content: String::new(),
            reasoning_content: None,
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    fn apply_content_delta(&mut self, delta: &str) {
        self.content += delta
    }

    fn apply_reasoning_delta(&mut self, delta: &str) {
        *self.reasoning_content.get_or_insert_with(|| delta.into()) += delta
    }

    fn apply_tool_delta(&mut self, delta: Value) {
        todo!()
    }

    // Input: delta string from api
    pub fn apply_delta(&mut self, delta: &str) {
        // let parsed = todo!()
    }
}

#[derive(Clone, Debug, Default)]
struct ToolCall {
    tool_name: String,
    id: String,
    arguments_string: String, // arguments arrive as a json string
}

impl ToolCall {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "type": "function",
            "function": {
                "name": self.tool_name,
                "arguments": self.arguments_string
            }
        })
    }
}

/**************** Tool description ****************/

#[derive(Clone, Debug)]
struct ToolParameter {
    name: &'static str,
    ptype: &'static str, // string, number or boolean
    description: Option<&'static str>,
    is_required: bool,
}

#[derive(Clone, Debug)]
struct ToolDefinition {
    // tool_type: String, // Function
    name: &'static str,
    description: Option<&'static str>,
    parameters: &'static [ToolParameter],
    // callback:
}

impl ToolDefinition {
    fn to_json(&self) -> Value {
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

fn print_message(message: &Message) {
    println!("{}\n==========\n", message.role);
    if let Some(reasoning) = &message.reasoning_content {
        println!("{}", reasoning.to_string().blue().italic())
    }
    println!("{}", message.content);
}

fn build_request(config: &Config, tools: &[ToolDefinition], history: &[Message]) -> String {
    let mut req = Map::new();

    req.insert("model".into(), json!(config.model_name));
    req.insert("max_tokens".into(), json!(10_000));
    req.insert("stream".into(), json!(config.stream));
    req.insert(
        "messages".into(),
        Value::Array(history.iter().map(|m| m.to_json()).collect()),
    );
    req.insert(
        "tools".into(),
        Value::Array(tools.iter().map(|t| t.to_json()).collect()),
    );

    Value::Object(req).to_string()
}

fn generate_response(
    config: &Config,
    tools: &[ToolDefinition],
    history: &[Message],
) -> Option<Message> {
    let request_body = build_request(config, tools, history);

    // println!("{request_body}");

    let res = reqwest::blocking::Client::new()
        .post(&config.url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", config.api_key))
        .body(request_body)
        .timeout(Duration::from_secs(60))
        .send();

    match res {
        Ok(mut resp) => {
            if !config.stream {
                println!("Status: {}", resp.status());
                let body = resp.text().unwrap_or_default();
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
                println!("\n{}", serde_json::to_string_pretty(&parsed).unwrap());
                // println!("{}", parsed["choices"][0]["message"]["content"].to_string());
                let message_content = parsed["choices"][0]["message"]["content"].as_str().unwrap();
                Some(Message {
                    role: "assistant".to_string(),
                    content: message_content.to_string(),
                    reasoning_content: parsed["choices"][0]["message"]["reasoning_content"]
                        .as_str()
                        .map(String::from),
                    ..Default::default()
                })
            } else {
                /* Streaming */
                let mut buffer_count = 0;
                let mut buffer = [0u8; 1024];
                let mut string = String::new();
                let mut message = Message::empty();

                loop {
                    let n_read = resp.read(&mut buffer[buffer_count..]).unwrap();
                    // println!("n read = {}", n_read);
                    buffer_count += n_read;

                    if let Some(pos) = buffer[0..buffer_count].iter().position(|&b| b == b'\n') {
                        string.push_str(&String::from_utf8_lossy(&buffer[0..pos]));
                        buffer.rotate_left(pos + 1);
                        buffer_count -= pos + 1;

                        // HANDLE DELTA HERE
                        if string.len() > 5 {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&string[5..]) {
                                // println!("PARSED!");
                                let delta = &parsed["choices"][0]["delta"];
                                // println!("{delta}");
                                if let Some(role) = delta.get("role") {
                                    message.role += role.as_str().unwrap_or_default();
                                }
                                if let Some(content) = delta.get("reasoning_content") {
                                    print!("{}", content.as_str().unwrap_or_default());
                                    std::io::stdout().flush().unwrap();
                                    message.apply_reasoning_delta(content.as_str().unwrap_or_default());
                                }
                                if let Some(content) = delta.get("content") {
                                    print!("{}", content.as_str().unwrap_or_default());
                                    std::io::stdout().flush().unwrap();
                                    message.apply_content_delta(content.as_str().unwrap_or_default());
                                }
                                if let Some(Value::Array(tool_calls)) = delta.get("tool_calls") {
                                    for toolcall in tool_calls {
                                        //TODO: put this in separate function and use ? operator to get index
                                        if let Some(index) = toolcall.get("index") {
                                            let index = index.as_u64().unwrap() as usize;
                                            println!("Index = {index}");
                                            if message.tool_calls.len() <= index {
                                                message.tool_calls.resize_with(index+1, || ToolCall::default())
                                            }
                                            if let Some(Value::String(name)) = toolcall["function"].get("name") {
                                                message.tool_calls[index].tool_name += name
                                            }
                                            if let Some(Value::String(id)) = toolcall.get("id") {
                                                message.tool_calls[index].id += id
                                            }
                                            if let Some(Value::String(argstring)) = toolcall["function"].get("arguments") {
                                                message.tool_calls[index].arguments_string += argstring
                                            }
                                        }
                                    }
                                    print!("{:?}", tool_calls);
                                    std::io::stdout().flush().unwrap();
                                }
                            }
                        }
                        string = String::new();
                    } else {
                        string.push_str(&String::from_utf8_lossy(&buffer[0..buffer_count]));
                        buffer_count = 0;
                    }
                    if n_read == 0 {
                        break;
                    }
                }
                // println!("New message! {:?}", message);
                // todo!()
                Some(message)
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            None
        }
    }
}

fn main() {
    let config = Config {
        url: "http://100.95.123.125:8080/v1/chat/completions".into(),
        model_name: "Qwen3.6-35B-A3B-UD-Q5_K_XL.gguf".into(),
        api_key: "none".into(),
        stream: true,
        preserve_thinking: true,
        truncate_long_tool_results_after_turn_finished: false,
    };

    let mut thread: Vec<Message> = vec![];

    let mut rl = rustyline::DefaultEditor::new().unwrap();

    let tools = &[ToolDefinition {
        name: "Shell",
        description: Some(
            "Run a shell/terminal command. Shell state is not preserved between calls of this tool.",
        ),
        parameters: &[ToolParameter {
            ptype: "string",
            name: "command",
            description: Some("Command to run in default OS shell."),
            is_required: true,
        }],
    }];

    loop {
        let input = rl.readline(">> ");
        match input {
            Ok(user_message) => {
                thread.push(Message {
                    role: "user".into(),
                    content: user_message,
                    ..Message::empty()
                });

                /* Response loop until no tool calls */
                loop {
                    let response = generate_response(&config, tools, &thread).unwrap();
                    let num_tool_calls = response.tool_calls.len();
                    // print_message(&response);

                    /* Execute tool calls now and push them to the history */
                    let mut tool_results = vec![];
                    for tool_call in &response.tool_calls {
                        if tool_call.tool_name == "Shell" {
                            tool_results.push(Message {
                                role: "tool".into(),
                                tool_call_id: Some(tool_call.id.clone()),
                                content: run_shell_command(&serde_json::from_str::<Value>(&tool_call.arguments_string).unwrap()["command"].as_str().unwrap()),
                                ..Default::default()
                            })
                        }
                    }

                    thread.push(response);
                    thread.append(&mut tool_results);
                    if num_tool_calls == 0 {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}
