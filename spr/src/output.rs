/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::{error::Result, jj::PreparedCommit, message::MessageSection};

pub enum Icons {
    Error,
    Info,
    Key,
    Land,
    OK,
    Question,
    Refresh,
    Sparkle,
    Stop,
    Wave,
}

fn icon_to_string(icon: Icons) -> &'static str {
    match icon {
        Icons::Error => "💔",
        Icons::Key => "🔑",
        Icons::Land => "🛬",
        Icons::OK => "✅",
        Icons::Question => "❓",
        Icons::Info => "❕",
        Icons::Refresh => "🔁",
        Icons::Sparkle => "✨",
        Icons::Stop => "🛑",
        Icons::Wave => "👋",
    }
}

pub fn output<S>(icon: Icons, text: S) -> Result<()>
where
    S: AsRef<str>,
{
    let term = console::Term::stdout();

    let bullet = format!("  {}  ", icon_to_string(icon));
    let indent = console::measure_text_width(&bullet);
    let indent_string = " ".repeat(indent);
    let options = textwrap::Options::new((term.size().1 as usize) - indent * 2)
        .initial_indent(&bullet)
        .subsequent_indent(&indent_string)
        .break_words(false)
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation);

    term.write_line(&textwrap::wrap(text.as_ref().trim(), &options).join("\n"))?;
    Ok(())
}

pub fn write_commit_title(prepared_commit: &PreparedCommit) -> Result<()> {
    let term = console::Term::stdout();
    term.write_line(&format!(
        "{} {}",
        console::style(&prepared_commit.short_id).italic(),
        console::style(
            prepared_commit
                .message
                .get(&MessageSection::Title)
                .map(|s| &s[..])
                .unwrap_or("(untitled)"),
        )
        .yellow()
    ))?;
    Ok(())
}
