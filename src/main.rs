use colored::Colorize;
use serde::{self, Deserialize, Serialize};
use serde_json::{self, Map, Value, json};
use std::error::Error;
use std::io::{Read, Write};
use std::{collections::HashMap, time::Duration};

mod utils;
use utils::*;

/* Chat-completions based agent :/ I really dislike the completions API.
 * This agent supports only function type tool calls with no parameter nesting
 */

#[derive(Clone, Debug)]
struct Config {
    url: String,
    model_name: String,
    api_key: String,
    stream: bool,
    preserve_thinking: bool,
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
    pub fn to_json(&self, config: &Config) -> Value {
        let mut message = Map::new();
        message.insert("role".into(), json!(self.role));
        message.insert("content".into(), json!(self.content));
        if config.preserve_thinking {
            self.reasoning_content
                .as_ref()
                .map(|r| message.insert("reasoning_content".into(), json!(r)));
        }
        if self.tool_calls.len() > 0 {
            message.insert(
                "tool_calls".into(),
                self.tool_calls.iter().map(ToolCall::to_json).collect(),
            );
        }
        Value::Object(message)
    }

    fn apply_content_delta(&mut self, delta: &str) {
        self.content += delta
    }

    fn apply_reasoning_delta(&mut self, delta: &str) {
        *self.reasoning_content.get_or_insert_with(|| delta.into()) += delta
    }

    fn apply_tool_delta(&mut self, call: &Value) {
        if let Some(index) = call.get("index") {
            let index = index.as_u64().unwrap() as usize;
            if self.tool_calls.len() <= index {
                self.tool_calls.resize_with(index + 1, Default::default)
            }
            if let Some(Value::String(name)) = call["function"].get("name") {
                self.tool_calls[index].tool_name += name
            }
            if let Some(Value::String(id)) = call.get("id") {
                self.tool_calls[index].id += id
            }
            if let Some(Value::String(arg)) = call["function"].get("arguments") {
                self.tool_calls[index].arguments_string += arg
            }
        }
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

fn build_request(config: &Config, tools: &[ToolDefinition], history: &[Message]) -> String {
    let mut req = Map::new();

    req.insert("model".into(), json!(config.model_name));
    req.insert("max_tokens".into(), json!(10_000));
    req.insert("stream".into(), json!(config.stream));
    req.insert(
        "messages".into(),
        Value::Array(history.iter().map(|m| m.to_json(config)).collect()),
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
) -> Result<Message, Box<dyn Error>> {
    let request_body = build_request(config, tools, history);

    let resp = reqwest::blocking::Client::new()
        .post(&config.url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", config.api_key))
        .body(request_body)
        .timeout(Duration::from_secs(3000))
        .send()?;

    if !config.stream {
        let body = resp.text().unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(&body)?;
        Ok(Message {
            role: "assistant".to_string(),
            content: parsed["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .into(),
            reasoning_content: parsed["choices"][0]["message"]["reasoning_content"]
                .as_str()
                .map(String::from),
            ..Default::default()
        })
    } else {
        /* Streaming */
        let mut message = Message::default();
        let mut stream = SplitStream::<_, 256>::new(resp, b'\n');

        while let Ok(update) = stream.next()
            && let Ok(string) = String::from_utf8(update)
        {
            if string.len() < 5 {
                continue;
            }

            /* Handle delta */
            if let Ok(parsed) = serde_json::from_str::<Value>(&string[5..]) {
                if let Some(finish_reason) = parsed["choices"][0]["finish_reason"].as_str()
                    && (finish_reason == "stop" || finish_reason == "tool_calls")
                {
                    println!("STOPPED!");
                    break;
                }
                let delta = &parsed["choices"][0]["delta"];
                if let Some(role) = delta.get("role") {
                    message.role += role.as_str().unwrap_or_default();
                }
                if let Some(content) = delta.get("reasoning_content") {
                    print_and_flush(content.as_str().unwrap_or_default());
                    message.apply_reasoning_delta(content.as_str().unwrap_or_default());
                }
                if let Some(content) = delta.get("content") {
                    print_and_flush(content.as_str().unwrap_or_default());
                    message.apply_content_delta(content.as_str().unwrap_or_default());
                }
                if let Some(Value::Array(tool_calls)) = delta.get("tool_calls") {
                    for call in tool_calls {
                        message.apply_tool_delta(&call);
                    }
                }
            }
        }
        Ok(message)
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

    let mut readline = rustyline::DefaultEditor::new().unwrap();

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
        let input = readline.readline(">> ");
        match input {
            Ok(user_message) => {
                thread.push(Message {
                    role: "user".into(),
                    content: user_message,
                    ..Default::default()
                });

                /* Response loop until no tool calls */
                loop {
                    let response = generate_response(&config, tools, &thread).unwrap();
                    let num_tool_calls = response.tool_calls.len();

                    /* Execute tool calls now and push them to the history */
                    let mut tool_results = vec![];
                    for tool_call in &response.tool_calls {
                        if tool_call.tool_name == "Shell" {
                            tool_results.push(Message {
                                role: "tool".into(),
                                tool_call_id: Some(tool_call.id.clone()),
                                content: run_shell_command(
                                    &serde_json::from_str::<Value>(&tool_call.arguments_string)
                                        .unwrap()["command"]
                                        .as_str()
                                        .unwrap(),
                                ),
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
