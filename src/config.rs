use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperASRConfig {
    pub url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "platform")]
pub enum AsrConfig {
    Whisper(WhisperASRConfig),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Provider {
    OpenAI,
    ByteFuture,
    Groq,
    GLM,
    Custom,
}

impl Provider {
    fn label(&self) -> &str {
        match self {
            Provider::OpenAI => "OpenAI",
            Provider::ByteFuture => "ByteFuture",
            Provider::Groq => "Groq",
            Provider::GLM => "GLM",
            Provider::Custom => "Custom",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" => Some(Provider::OpenAI),
            "bytefuture" => Some(Provider::ByteFuture),
            "groq" => Some(Provider::Groq),
            "glm" => Some(Provider::GLM),
            "custom" => Some(Provider::Custom),
            _ => None,
        }
    }

    fn defaults(&self) -> (&str, &str) {
        // (url, model)
        match self {
            Provider::OpenAI => (
                "https://api.openai.com/v1/audio/transcriptions",
                "whisper-1",
            ),
            Provider::ByteFuture => (
                "https://models.bytefuture.ai/v1/audio/transcriptions",
                "groq/whisper-large-v3",
            ),
            Provider::Groq => (
                "https://api.groq.com/openai/v1/audio/transcriptions",
                "whisper-large-v3-turbo",
            ),
            Provider::GLM => (
                "https://open.bigmodel.cn/api/paas/v4/audio/transcriptions",
                "glm-asr-2512",
            ),
            Provider::Custom => ("", ""),
        }
    }

    pub fn all() -> Vec<Provider> {
        vec![
            Provider::OpenAI,
            Provider::ByteFuture,
            Provider::Groq,
            Provider::GLM,
            Provider::Custom,
        ]
    }
}

pub fn config_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".vibekeys").join("config.toml"))
}

pub fn load_config() -> Option<AsrConfig> {
    let path = config_path()?;
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

pub fn save_config(config: &AsrConfig) -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    let dir = home.join(".vibekeys");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    println!("Config saved to: {}", path.display());
    Ok(())
}

pub fn run_setup(platform: Option<String>, api_key: Option<String>) -> anyhow::Result<()> {
    // If platform is provided, use it directly
    if let Some(platform_str) = platform {
        let provider = Provider::from_str(&platform_str)
            .ok_or_else(|| anyhow::anyhow!("Unknown platform: {}", platform_str))?;

        let existing = load_config();
        let (default_url, default_model) = provider.defaults();

        // Get URL, model, lang from existing config or defaults
        let (url, model, lang) = if let Some(AsrConfig::Whisper(cfg)) = &existing {
            (
                if provider == Provider::Custom {
                    cfg.url.clone()
                } else {
                    default_url.to_string()
                },
                if cfg.model.is_empty() {
                    default_model.to_string()
                } else {
                    cfg.model.clone()
                },
                cfg.lang.clone(),
            )
        } else {
            (default_url.to_string(), default_model.to_string(), String::new())
        };

        // Use provided api_key or fall back to existing
        let api_key = if let Some(key) = api_key {
            key
        } else if let Some(AsrConfig::Whisper(cfg)) = &existing {
            cfg.api_key.clone()
        } else {
            String::new()
        };

        let config = AsrConfig::Whisper(WhisperASRConfig {
            url,
            api_key,
            lang,
            model,
            prompt: String::new(),
        });

        return save_config(&config);
    }

    // Interactive mode
    let existing = load_config();

    // Get existing values if available
    let (_existing_url, existing_model, existing_api_key, existing_lang) =
        if let Some(AsrConfig::Whisper(cfg)) = &existing {
            (
                Some(cfg.url.clone()),
                Some(cfg.model.clone()),
                Some(cfg.api_key.clone()),
                Some(cfg.lang.clone()),
            )
        } else {
            (None, None, None, None)
        };

    println!("=== Vibekeys ASR Setup ===\n");

    // Select provider
    let providers = Provider::all();
    let provider_index = dialoguer::Select::new()
        .with_prompt("Select ASR provider")
        .items(&providers.iter().map(|p| p.label()).collect::<Vec<_>>())
        .default(0)
        .interact()?;

    let provider = providers[provider_index];
    let (default_url, default_model) = provider.defaults();

    // Input URL
    let url = if provider == Provider::Custom {
        dialoguer::Input::new()
            .with_prompt("ASR URL")
            .allow_empty(false)
            .interact()?
    } else {
        default_url.to_string()
    };

    // Input Model (use default from provider, or existing value)
    let model = if existing_model.as_ref().is_some_and(|m| !m.is_empty()) {
        existing_model.unwrap()
    } else {
        default_model.to_string()
    };

    // Input API Key
    let api_key = dialoguer::Input::new()
        .with_prompt("API Key")
        .allow_empty(true)
        .with_initial_text(existing_api_key.as_deref().unwrap_or(""))
        .interact()?;

    // Input Language
    let lang = dialoguer::Input::new()
        .with_prompt("Language (leave empty for auto-detect)")
        .allow_empty(true)
        .with_initial_text(existing_lang.as_deref().unwrap_or(""))
        .interact()?;

    let config = AsrConfig::Whisper(WhisperASRConfig {
        url,
        api_key,
        lang,
        model,
        prompt: String::new(),
    });

    save_config(&config)?;

    println!("\nSetup complete!");
    Ok(())
}
