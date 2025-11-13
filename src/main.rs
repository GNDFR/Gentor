use async_openai::{
    config::OpenAIConfig,
    types::{ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent, CreateChatCompletionRequestArgs},
    Client
};
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io,
    path::PathBuf,
    time::{Duration, Instant},
};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};

const SETTINGS_FILE: &str = "settings.json";

#[derive(Serialize, Deserialize)]
struct Settings {
    provider: String,
    model: String,
    api_key: String,
    base_url: String,
}

#[derive(Clone)]
enum AppState {
    Chat,
    Settings,
}

struct App {
    state: AppState,
    input: String,
    messages: Vec<String>,
    settings: Settings,
    settings_input: Vec<String>,
    settings_focus: usize,
    confirm_save: bool,
    last_confirm: Option<Instant>,
    just_entered_settings: bool,
}

impl App {
    fn new(settings: Settings) -> Self {
        let settings_input = vec![
            settings.provider.clone(),
            settings.model.clone(),
            settings.api_key.clone(),
            settings.base_url.clone(),
        ];
        Self {
            state: AppState::Chat,
            input: String::new(),
            messages: vec!["🧠 Gentor ready! Type your message or '/setting' to edit config.".to_string()],
            settings,
            settings_input,
            settings_focus: 0,
            confirm_save: false,
            last_confirm: None,
            just_entered_settings: false,
        }
    }

    fn save_settings(&mut self) -> Result<()> {
        self.settings.provider = self.settings_input[0].clone();
        self.settings.model = self.settings_input[1].clone();
        self.settings.api_key = self.settings_input[2].clone();
        self.settings.base_url = self.settings_input[3].clone();
        let json = serde_json::to_string_pretty(&self.settings)?;
        fs::write(SETTINGS_FILE, json)?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    ensure_settings_file()?;
    let settings: Settings = serde_json::from_str(&fs::read_to_string(SETTINGS_FILE)?)?;

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(settings);

    loop {
        if let Some(time) = app.last_confirm {
            if time.elapsed() > Duration::from_secs(2) {
                app.confirm_save = false;
                app.last_confirm = None;
            }
        }
        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.state.clone() {
                    AppState::Chat => {
                        if key.kind == KeyEventKind::Press {
                            match key.code {
                                KeyCode::Enter => {
                                    let input = app.input.trim();
                                    if input == "/exit" {
                                        break;
                                    } else if input == "/setting" {
                                        app.state = AppState::Settings;
                                        app.confirm_save = false;
                                        app.last_confirm = None;
                                        app.just_entered_settings = true;
                                    } else if !app.input.is_empty() {
                                        let config = OpenAIConfig::new()
                                            .with_api_key(app.settings.api_key.clone())
                                            .with_api_base(app.settings.base_url.clone());
                                        let client = Client::with_config(config);
                                        match run_agent(&client, &app.settings.model, &app.input).await {
                                            Ok(response) => {
                                                app.messages.push(format!("> {}", app.input));
                                                app.messages.push(format!("🤖 {}", response.trim()));
                                            }
                                            Err(e) => {
                                                app.messages.push(format!("⚠️ Error: {}", e));
                                            }
                                        }
                                        app.input.clear();
                                    }
                                }
                                KeyCode::Char(c) => {
                                    app.input.push(c);
                                }
                                KeyCode::Backspace => {
                                    app.input.pop();
                                }
                                KeyCode::Esc => break,
                                _ => {}
                            }
                        }
                    }
                    AppState::Settings => {
                        if key.kind == KeyEventKind::Press {
                            match key.code {
                                KeyCode::Enter => {
                                    if app.just_entered_settings {
                                        app.just_entered_settings = false;
                                    } else if app.confirm_save {
                                        if let Err(e) = app.save_settings() {
                                            app.messages.push(format!("⚠️ Failed to save settings: {}", e));
                                        } else {
                                            app.messages.push("✅ Settings saved!".to_string());
                                        }
                                        app.confirm_save = false;
                                        app.last_confirm = None;
                                        app.state = AppState::Chat;
                                    } else {
                                        app.confirm_save = true;
                                        app.last_confirm = Some(Instant::now());
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if app.settings_focus < 4 {
                                        app.settings_input[app.settings_focus].push(c);
                                    }
                                }
                                KeyCode::Backspace => {
                                    if app.settings_focus < 4 {
                                        app.settings_input[app.settings_focus].pop();
                                    }
                                }
                                KeyCode::Up => {
                                    if app.settings_focus > 0 {
                                        app.settings_focus -= 1;
                                    }
                                }
                                KeyCode::Down => {
                                    if app.settings_focus < 3 {
                                        app.settings_focus += 1;
                                    }
                                }
                                KeyCode::Esc => {
                                    app.confirm_save = false;
                                    app.last_confirm = None;
                                    app.state = AppState::Chat;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let size = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)].as_ref())
        .split(size);

    let messages_text = app.messages.join("\n");
    let messages_paragraph = Paragraph::new(messages_text)
        .block(Block::default().borders(Borders::ALL).title("Chat"))
        .wrap(tui::widgets::Wrap { trim: false });

    f.render_widget(messages_paragraph, chunks[0]);

    match app.state {
        AppState::Chat => {
            let input = Paragraph::new(app.input.as_str())
                .style(Style::default().fg(Color::Yellow))
                .block(Block::default().borders(Borders::ALL).title("Input (Enter: send, /setting: config, /exit: exit)"));
            f.render_widget(input, chunks[1]);
            f.set_cursor(chunks[1].x + app.input.len() as u16 + 1, chunks[1].y + 1);
        }
        AppState::Settings => {
            let settings_block = Block::default().borders(Borders::ALL).title("Settings Editor");
            f.render_widget(Clear, size);
            f.render_widget(settings_block, size);

            let inner_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ])
                .margin(2)
                .split(size);

            let save_text = if app.confirm_save { "Press one more to save" } else { "Press Enter to Save" };
            let fields = ["Provider", "Model", "API Key", "Base URL", save_text];

            for i in 0..5 {
                let style = if i == app.settings_focus && i < 4 {
                    Style::default().fg(Color::Black).bg(Color::White)
                } else {
                    Style::default()
                };
                let text = if i < 4 {
                    app.settings_input[i].as_str()
                } else {
                    save_text
                };
                let para = Paragraph::new(text)
                    .style(style)
                    .block(Block::default().borders(Borders::ALL).title(fields[i]));
                f.render_widget(para, inner_chunks[i]);
            }
            if app.settings_focus < 4 {
                f.set_cursor(
                    inner_chunks[app.settings_focus].x + app.settings_input[app.settings_focus].len() as u16 + 1,
                    inner_chunks[app.settings_focus].y + 1,
                );
            }
        }
    }
}

async fn run_agent(client: &Client<OpenAIConfig>, model: &str, prompt: &str) -> Result<String> {
    use async_openai::types::{ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestSystemMessageContent};

    let system_message = ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
        content: ChatCompletionRequestSystemMessageContent::Text("You are Gentor, an expert coding assistant. Help with programming tasks, code generation, debugging, and explanations. Be concise and helpful.".to_string()),
        name: None,
    });

    let user_message = ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
        content: ChatCompletionRequestUserMessageContent::Text(prompt.to_string()),
        name: None,
    });

    let req = CreateChatCompletionRequestArgs::default()
        .model(model)
        .messages([system_message, user_message])
        .build()?;

    let res = client.chat().create(req).await?;
    Ok(res.choices[0].message.content.clone().unwrap_or_default())
}

fn ensure_settings_file() -> Result<()> {
    let path = PathBuf::from(SETTINGS_FILE);
    if !path.exists() {
        println!("🪄 settings.json이 없습니다. 새로 생성합니다...");
        let example = Settings {
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_key: "sk-your-api-key".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
        };
        let json = serde_json::to_string_pretty(&example)?;
        fs::write(&path, json)?;
        println!("✅ settings.json이 생성되었습니다. API 키를 입력 후 다시 실행하세요.");
        std::process::exit(0);
    }
    Ok(())
}
