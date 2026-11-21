use super::style::{FAINT, Rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brand {
    pub label: &'static str,
    pub glyph: &'static str,
    pub color: Rgb,
}

const CLAUDE: Rgb = Rgb(0xD9, 0x77, 0x57);
const OPENAI: Rgb = Rgb(0xE8, 0xE8, 0xE8);
const LANGGRAPH: Rgb = Rgb(0x7F, 0xC8, 0xC0);
const LITELLM: Rgb = Rgb(0x8A, 0xB4, 0xF8);
const GOOGLE: Rgb = Rgb(0x7B, 0xAA, 0xF7);
const CREWAI: Rgb = Rgb(0xFF, 0x7A, 0x6E);
const PYDANTIC: Rgb = Rgb(0xE9, 0x5C, 0x8A);

const fn brand(label: &'static str, glyph: &'static str, color: Rgb) -> Brand {
    Brand {
        label,
        glyph,
        color,
    }
}

pub fn harness(id: &str) -> Brand {
    match id {
        "claude-agent-sdk" => brand("Claude Agent SDK", "✻", CLAUDE),
        "claude_code" | "claude-code" => brand("Claude Code", "✻", CLAUDE),
        "openai-agents" => brand("OpenAI Agents", "◎", OPENAI),
        "codex" | "openai-codex" => brand("Codex", "◎", OPENAI),
        "opencode" => brand("OpenCode", "◇", FAINT),
        "pi" => brand("Pi", "π", FAINT),
        "tool_loop" => brand("LiteLLM Tool Loop", "≋", LITELLM),
        "langgraph" => brand("LangGraph", "⬡", LANGGRAPH),
        "langchain" => brand("LangChain", "⬡", LANGGRAPH),
        "deepagents" => brand("Deep Agents", "⬡", LANGGRAPH),
        "litellm" => brand("LiteLLM", "≋", LITELLM),
        "google-adk" => brand("Google ADK", "◆", GOOGLE),
        "crewai" => brand("CrewAI", "◈", CREWAI),
        "pydantic-ai" => brand("Pydantic AI", "▲", PYDANTIC),
        _ => brand("Custom", "◇", FAINT),
    }
}

pub fn model(name: &str) -> Brand {
    let base = name.rsplit('/').next().unwrap_or_default();
    match base.split(['-', '.', ':']).next().unwrap_or_default() {
        "claude" | "sonnet" | "opus" | "haiku" => brand("Anthropic", "✻", CLAUDE),
        "gpt" | "o1" | "o3" | "o4" | "codex" => brand("OpenAI", "◎", OPENAI),
        "gemini" | "gemma" => brand("Google", "◆", GOOGLE),
        _ => brand("Unknown", "◇", FAINT),
    }
}
