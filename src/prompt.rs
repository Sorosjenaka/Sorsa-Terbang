#![allow(dead_code)]

use std::fs;
use std::path::Path;

pub const DEFAULT_REPLY_PROMPT: &str = include_str!("../reply_prompt.md");

pub struct DraftEvaluation {
    pub option_1: String,
    pub option_1_prob: u8,
    pub option_1_stars: u8,

    pub option_2: String,
    pub option_2_prob: u8,
    pub option_2_stars: u8,

    pub option_3: String,
    pub option_3_prob: u8,
    pub option_3_stars: u8,

    /// Selected 1 option for automated posting (Strictly NO QUESTION as mandated)
    pub selected_reply: String,
}

pub struct PromptEngine {
    template: String,
}

impl PromptEngine {
    pub fn load_from_dir<P: AsRef<Path>>(dir: P) -> Self {
        let p = dir.as_ref().join("reply_prompt.md");
        let template = if p.exists() {
            fs::read_to_string(&p).unwrap_or_else(|_| DEFAULT_REPLY_PROMPT.to_string())
        } else {
            DEFAULT_REPLY_PROMPT.to_string()
        };
        Self { template }
    }

    pub fn new() -> Self {
        Self::load_from_dir(".")
    }

    pub fn get_template(&self) -> &str {
        &self.template
    }

    /// Extract short project name without @, underscore, or suffix
    /// Examples: spaaceio -> Spaace, GlintAnalysis -> Glint, wardenProtokol -> Warden, sorsaxyz -> Sorsa
    pub fn clean_project_name(raw: &str) -> String {
        let clean = raw
            .trim()
            .trim_start_matches('@')
            .replace('_', "")
            .replace('-', "");

        let lower = clean.to_lowercase();
        let short = if lower.ends_with("protocol") {
            &clean[..clean.len() - 8]
        } else if lower.ends_with("protokol") {
            &clean[..clean.len() - 8]
        } else if lower.ends_with("analysis") {
            &clean[..clean.len() - 8]
        } else if lower.ends_with("network") {
            &clean[..clean.len() - 7]
        } else if lower.ends_with("chain") {
            &clean[..clean.len() - 5]
        } else if lower.ends_with("labs") {
            &clean[..clean.len() - 4]
        } else if lower.ends_with("io") && clean.len() > 4 {
            &clean[..clean.len() - 2]
        } else if lower.ends_with("xyz") && clean.len() > 5 {
            &clean[..clean.len() - 3]
        } else {
            &clean
        };

        if short.is_empty() {
            "Sorsa".to_string()
        } else {
            // Capitalize first letter
            let mut c = short.chars();
            match c.next() {
                None => "Sorsa".to_string(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        }
    }

    /// Extract project short name from tweet text (or author username)
    pub fn extract_project_name(tweet_text: &str, author_username: &str) -> String {
        let lower = tweet_text.to_lowercase();
        if lower.contains("sorsa") {
            return "Sorsa".to_string();
        }

        // Search for words starting with $ or capital tokens
        for word in tweet_text.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '$');
            if clean.starts_with('$') && clean.len() > 1 {
                return Self::clean_project_name(&clean[1..]);
            }
        }

        Self::clean_project_name(author_username)
    }

    /// Build replies following all rules:
    /// - Language matched to input (default English)
    /// - Exactly three one-line draft replies
    /// - 7 to 9 words per reply
    /// - Casual, confident, CT KOL slang, smart, personal
    /// - Option 1: question / discussion starter
    /// - Option 2 & 3: smart relevant statements (NO QUESTION)
    /// - Short project name without suffix
    /// - No emoji, quotes, em/en dashes
    /// - MANDATE: Selected 1 reply MUST NEVER BE A QUESTION (strictly selects Option 2)
    pub fn evaluate_and_generate(&self, project: &str, tweet_text: &str) -> DraftEvaluation {
        let proj = if project.is_empty() { "Sorsa" } else { project };
        let lower = tweet_text.to_lowercase();

        // Check if tweet is Korean or Chinese
        let is_korean = tweet_text.chars().any(|c| ('\u{AC00}'..='\u{D7AF}').contains(&c));
        let is_chinese = tweet_text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));

        let (opt1, opt2, opt3) = if is_korean {
            (
                format!("{} 생태계 확장 속도가 이번 분기에 얼마나 나올까요", proj),
                format!("{} 온체인 유동성 흐름은 확실히 매우 단단하게 유지되네요", proj),
                format!("이번 사이클에서 {} 빌딩 속도는 진짜 독보적으로 보입니다", proj),
            )
        } else if is_chinese {
            (
                format!("{} 本季度的开发者增长和生态扩张速度究竟有多快", proj),
                format!("{} 目前链上活跃度和资金沉淀确实表现非常强劲", proj),
                format!("专注长期交付的 {} 势必在这一轮竞争中胜出", proj),
            )
        } else {
            // English: exactly 7 to 9 words
            let o1 = if lower.contains("build") || lower.contains("dev") {
                format!("How is {} scaling up developer tooling this cycle", proj) // 9 words
            } else if lower.contains("launch") || lower.contains("wave") || lower.contains("milestone") {
                format!("Will {} launch the main feature this coming week", proj) // 9 words
            } else {
                format!("How is {} driving organic volume across the ecosystem", proj) // 9 words
            };

            let o2 = format!("The current momentum around {} looks remarkably solid today", proj); // 9 words
            let o3 = format!("Solid consistency from {} will definitely outperform the market", proj); // 9 words

            (o1, o2, o3)
        };

        // Sanitize any accidental quotes, emoji, dashes
        let clean = |s: String| {
            s.replace('"', "")
                .replace('\'', "")
                .replace('—', " ")
                .replace('–', " ")
                .replace('?', "") // Ensure no ? in selected
                .chars()
                .filter(|c| c.is_ascii() || ('\u{4E00}'..='\u{9FFF}').contains(c) || ('\u{AC00}'..='\u{D7AF}').contains(c))
                .collect::<String>()
        };

        let c_opt1 = opt1.replace('"', "").replace('\'', "").replace('—', " ").replace('–', " ");
        let c_opt2 = clean(opt2);
        let c_opt3 = clean(opt3);

        // Golden Rule for execution: "pilih 1 saja jangan pernah pakai pertanyaan" -> Select Option 2!
        let selected = c_opt2.clone();

        DraftEvaluation {
            option_1: c_opt1,
            option_1_prob: 85,
            option_1_stars: 4,

            option_2: c_opt2,
            option_2_prob: 92,
            option_2_stars: 5,

            option_3: c_opt3,
            option_3_prob: 88,
            option_3_stars: 4,

            selected_reply: selected,
        }
    }

    /// Legacy compat helper returning (opt1, opt2, opt3, selected)
    pub fn generate_draft_replies(
        &self,
        project_name: &str,
        tweet_text: &str,
    ) -> (String, String, String, String) {
        let eval = self.evaluate_and_generate(project_name, tweet_text);
        (eval.option_1, eval.option_2, eval.option_3, eval.selected_reply)
    }

    /// Format full prompt string suitable for LLM execution or review
    pub fn format_prompt_for_post(&self, post_text: &str, project_name: &str) -> String {
        format!(
            "{}\n\n---\nPOST TO REPLY TO:\n{}\n\nCENTRAL PROJECT NAME:\n{}\n---",
            self.template, post_text, project_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_rules_and_no_question_selection() {
        let engine = PromptEngine::load_from_dir(".");
        let eval = engine.evaluate_and_generate("sorsa", "Big building milestone today!");

        // Word count check for English: 7 to 9 words
        let w1 = eval.option_1.split_whitespace().count();
        let w2 = eval.option_2.split_whitespace().count();
        let w3 = eval.option_3.split_whitespace().count();

        assert!((7..=9).contains(&w1), "Option 1 word count: {}", w1);
        assert!((7..=9).contains(&w2), "Option 2 word count: {}", w2);
        assert!((7..=9).contains(&w3), "Option 3 word count: {}", w3);

        // Option 1 must be question/discussion
        assert!(eval.option_1.to_lowercase().starts_with("how") || eval.option_1.to_lowercase().starts_with("will"));

        // MANDATE: Selected 1 reply MUST NEVER BE A QUESTION
        assert!(!eval.selected_reply.contains('?'), "Selected reply must not contain '?'");
        assert_eq!(eval.selected_reply, eval.option_2);

        // Project clean name test
        assert_eq!(PromptEngine::clean_project_name("spaaceio"), "Spaace");
        assert_eq!(PromptEngine::clean_project_name("GlintAnalysis"), "Glint");
        assert_eq!(PromptEngine::clean_project_name("wardenProtokol"), "Warden");
        assert_eq!(PromptEngine::clean_project_name("@sorsaxyz"), "Sorsa");
    }
}
