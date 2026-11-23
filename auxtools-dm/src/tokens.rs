use dreammaker::lexer::{Punctuation, Token};

pub fn text_of<'a>(tokens: impl IntoIterator<Item = &'a Token>) -> String {
	let mut text = String::new();
	let mut previous: Option<&Token> = None;
	for token in tokens {
		if matches!(token, Token::Punct(Punctuation::Newline) | Token::DocComment(_)) {
			continue;
		}
		if previous.is_some_and(|previous| token.separate_from(previous)) {
			text.push(' ');
		}
		text += &token.to_string();
		previous = Some(token);
	}
	text
}
