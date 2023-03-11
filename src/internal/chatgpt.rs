// SPDX-License-Identifier: MIT
// Akira Moroo <retrage01@gmail.com> 2023

// Ask ChatGPT to generate cases for the given function.
// Use hyper to send a POST request to the ChatGPT API.

use std::collections::HashSet;

use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use hyper::{Body, Client, Request, Uri};
use hyper_tls::HttpsConnector;
use log::debug;
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use serde::{Deserialize, Serialize};
use syn::{parse_str, Ident, ItemFn, ItemMod};
use tokio::runtime::Runtime;

#[derive(Deserialize, Serialize, Debug)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct Chat {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatCompletion {
    id: String,
    object: String,
    created: u64,
    choices: Vec<ChatChoice>,
    usage: ChatUsage,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

async fn completion(chat: &mut Chat) -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is not set");
    let url = "https://api.openai.com/v1/chat/completions";
    let uri: Uri = url.parse()?;

    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);

    let body = Body::from(serde_json::to_string(&chat)?);

    let mut request = Request::new(body);

    *request.method_mut() = hyper::Method::POST;
    *request.uri_mut() = uri.clone();

    request
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", api_key)).unwrap(),
    );

    let response = client.request(request).await?;
    let body_bytes = hyper::body::to_bytes(response.into_body()).await?;
    let body_str = String::from_utf8(body_bytes.to_vec())?;

    let chat_completion: ChatCompletion = serde_json::from_str(&body_str)?;

    let content = chat_completion.choices[0].message.content.clone();

    debug!("ChatGPT created test case:\n{}", content);

    chat.messages.push(ChatMessage {
        role: "assistant".to_string(),
        content,
    });

    Ok(())
}

fn init_chat_messages(chat: &mut Chat, input: TokenStream) {
    chat.messages.push(ChatMessage {
        role: "system".to_string(),
        content: "You are a Rust expert that can generate perfect tests for the given function."
            .to_string(),
    });
    chat.messages.push(ChatMessage {
        role: "user".to_string(),
        content: format!("Read this Rust function:\n```rust\n{}\n```", input),
    });
}

fn generate_test_from(
    chat: &mut Chat,
    output: &mut proc_macro2::TokenStream,
    prompt: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let rt = Runtime::new()?;

    chat.messages.push(ChatMessage {
        role: "user".to_string(),
        content: prompt,
    });

    rt.block_on(completion(chat))?;

    let test_text = chat.messages[chat.messages.len() - 1].content.clone();
    // Remove the code block and remaining explanation text.
    // Extract the test case in the code block. Other parts are removed.
    let test_text = test_text
        .split("```rust")
        .nth(1)
        .ok_or(format!("No code block start found: {}", test_text))?
        .split("```")
        .next()
        .ok_or(format!("No code block end found: {}", test_text))?
        .trim()
        .to_string();

    let expanded = if let Ok(test_case) = parse_str::<ItemFn>(&test_text) {
        quote! {
            #test_case
        }
    } else if let Ok(test_case) = parse_str::<ItemMod>(&test_text) {
        quote! {
            #test_case
        }
    } else {
        return Err(format!(
            "Failed to parse the test case as a function or a module:\n{}\n",
            test_text
        )
        .into());
    };

    expanded.to_tokens(output);

    Ok(())
}

fn generate_test_from_test_name(
    chat: &mut Chat,
    output: &mut proc_macro2::TokenStream,
    test_name: Ident,
) -> Result<(), Box<dyn std::error::Error>> {
    generate_test_from(chat, output, format!("Write a test case `{}` for the function in Markdown code snippet style. Your response must start with code block '```rust'.", test_name))
}

fn generate_test_without_test_name(
    chat: &mut Chat,
    output: &mut proc_macro2::TokenStream,
) -> Result<(), Box<dyn std::error::Error>> {
    generate_test_from(chat, output, "Write a test case for the function as much as possible in Markdown code snippet style. Your response must start with code block '```rust'.".to_string())
}

pub fn generate_tests(
    input: TokenStream,
    test_names: HashSet<Ident>,
) -> Result<TokenStream, Box<dyn std::error::Error>> {
    let mut output = input.clone().into();

    let mut chat = Chat {
        model: "gpt-3.5-turbo".to_string(),
        messages: vec![],
    };

    init_chat_messages(&mut chat, input);

    if test_names.is_empty() {
        generate_test_without_test_name(&mut chat, &mut output)?;
    } else {
        for test_name in test_names {
            generate_test_from_test_name(&mut chat, &mut output, test_name)?;
        }
    }

    Ok(TokenStream::from(output))
}