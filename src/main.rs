use colored::Colorize;
use serde_json::{self, json};
use std::{collections::HashMap, time::Duration};

/* Chat-completions based agent :/ I really dislike the completions API */

#[derive(Clone, Debug, Default)]
struct Message {
    role: String,
    content: String,
    reasoning_content: Option<String>,
}

#[derive(Clone, Debug)]
struct CompletionsContext {
    url: String,
    model: String,
    messages: Vec<Message>,
}

impl CompletionsContext {
    fn new(url: impl Into<String>, model: impl Into<String>) -> Self {
        return Self {
            url: url.into(),
            model: model.into(),
            messages: vec![],
        };
    }

    fn add_message(&mut self, role: impl Into<String>, message: impl Into<String>) {
        self.messages.push(Message {
            role: role.into(),
            content: message.into(),
            ..Default::default()
        })
    }

    fn generate_response(&mut self) {
        let messages = self
            .messages
            .iter()
            .map(|m| {
                json!({
                    "role": &m.role,
                    "content": &m.content
                })
                .to_string()
            })
            .collect::<Vec<_>>();

        let request_body = format!(
            "{{\"model\":\"{}\",\"messages\":[{}],\"max_tokens\":10000, \"stream\": false}}",
            self.model,
            messages.join(",")
        );

        let res = reqwest::blocking::Client::new()
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer none")
            .body(request_body)
            .timeout(Duration::from_secs(60))
            .send();

        match res {
            Ok(resp) => {
                println!("Status: {}", resp.status());
                let body = resp.text().unwrap_or_default();
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
                println!("\n{}", serde_json::to_string_pretty(&parsed).unwrap());
                // println!("{}", parsed["choices"][0]["message"]["content"].to_string());
                let message_content = parsed["choices"][0]["message"]["content"].as_str().unwrap();
                self.messages.push(Message {
                    role: "assistant".to_string(),
                    content: message_content.to_string(),
                    reasoning_content: parsed["choices"][0]["message"]["reasoning_content"]
                        .as_str()
                        .map(String::from),
                });
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    fn print_latest(&self) {
        self.messages.last().map(|m| {
            println!("{}\n==========\n", m.role);
            if let Some(reasoning) = &m.reasoning_content {
                println!("{}", reasoning.to_string().blue().italic())
            }
            println!("{}", m.content);
        });
    }
}

fn main() {
    let mut chat = CompletionsContext::new(
        "http://100.95.123.125:8080/v1/chat/completions",
        "Qwen3.6-35B-A3B-UD-Q5_K_XL.gguf",
    );

    loop {
        let input = inquire::Text::new("What are you thinking about?").prompt();
        match input {
            Ok(user_message) => {
                chat.add_message("user", user_message);
                chat.generate_response();
                chat.print_latest();
            }
            _ => {}
        }
    }

    // Equivalent to:
    // curl http://localhost:8080/v1/chat/completions \
    //   -H "Content-Type: application/json" \
    //   -H "Authorization: Bearer none" \
    //   -d '{"model":"Qwen3.6-35B-A3B-UD-Q5_K_XL.gguf","messages":[{"role":"user","content":"hello"}],"max_tokens":100}'
}
