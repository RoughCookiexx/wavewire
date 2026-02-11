use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub struct App {
    pub running: bool,
}

impl App {
    pub fn new() -> Self {
        Self { running: true }
    }

    pub fn render(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .constraints([Constraint::Percentage(100)])
            .split(frame.area());

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan));

        let paragraph = Paragraph::new("Initializing audio engine...")
            .block(block);

        frame.render_widget(paragraph, chunks[0]);
    }
}
