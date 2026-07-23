use std::fmt;
use std::sync::{Arc, Mutex};

use crate::errors::{AntlrError, ConsoleErrorListener, ErrorListener};
use crate::vocabulary::Vocabulary;

#[derive(Clone)]
struct ErrorListenerSlot(Arc<Mutex<dyn for<'a> ErrorListener<dyn Recognizer + 'a> + Send>>);

impl ErrorListenerSlot {
    fn new<L>(listener: L) -> Self
    where
        L: for<'a> ErrorListener<dyn Recognizer + 'a> + Send + 'static,
    {
        Self(Arc::new(Mutex::new(listener)))
    }

    fn syntax_error(
        &self,
        recognizer: &(dyn Recognizer + '_),
        line: usize,
        column: usize,
        message: &str,
        error: Option<&AntlrError>,
    ) {
        self.0
            .lock()
            .expect("error listener lock poisoned")
            .syntax_error(recognizer, line, column, message, error);
    }
}

impl fmt::Debug for ErrorListenerSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ErrorListener")
    }
}

#[derive(Clone, Debug)]
pub struct RecognizerData {
    grammar_file_name: String,
    rule_names: Vec<String>,
    channel_names: Vec<String>,
    mode_names: Vec<String>,
    vocabulary: Vocabulary,
    state: isize,
    error_listeners: Vec<ErrorListenerSlot>,
}

impl RecognizerData {
    pub fn new(grammar_file_name: impl Into<String>, vocabulary: Vocabulary) -> Self {
        Self {
            grammar_file_name: grammar_file_name.into(),
            rule_names: Vec::new(),
            channel_names: Vec::new(),
            mode_names: Vec::new(),
            vocabulary,
            state: -1,
            error_listeners: vec![ErrorListenerSlot::new(ConsoleErrorListener)],
        }
    }

    #[must_use]
    pub fn with_rule_names(
        mut self,
        rule_names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.rule_names = rule_names.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn with_channel_names(
        mut self,
        channel_names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.channel_names = channel_names.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn with_mode_names(
        mut self,
        mode_names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.mode_names = mode_names.into_iter().map(Into::into).collect();
        self
    }

    /// Rule names owned by this recognizer's metadata.
    ///
    /// Also available through [`Recognizer::rule_names`]; this inherent
    /// accessor lets callers that already hold a `RecognizerData` field
    /// borrow rule names without borrowing the whole recognizer.
    #[must_use]
    pub fn rule_names(&self) -> &[String] {
        &self.rule_names
    }

    pub const fn state(&self) -> isize {
        self.state
    }

    pub const fn set_state(&mut self, state: isize) {
        self.state = state;
    }

    fn add_error_listener<L>(&mut self, listener: L)
    where
        L: for<'a> ErrorListener<dyn Recognizer + 'a> + Send + 'static,
    {
        self.error_listeners.push(ErrorListenerSlot::new(listener));
    }

    fn remove_error_listeners(&mut self) {
        self.error_listeners.clear();
    }

    fn notify_error_listeners(
        &self,
        recognizer: &dyn Recognizer,
        line: usize,
        column: usize,
        message: &str,
        error: Option<&AntlrError>,
    ) {
        for listener in &self.error_listeners {
            listener.syntax_error(recognizer, line, column, message, error);
        }
    }
}

pub trait Recognizer {
    fn data(&self) -> &RecognizerData;
    fn data_mut(&mut self) -> &mut RecognizerData;

    fn grammar_file_name(&self) -> &str {
        &self.data().grammar_file_name
    }

    fn rule_names(&self) -> &[String] {
        &self.data().rule_names
    }

    fn channel_names(&self) -> &[String] {
        &self.data().channel_names
    }

    fn mode_names(&self) -> &[String] {
        &self.data().mode_names
    }

    fn vocabulary(&self) -> &Vocabulary {
        &self.data().vocabulary
    }

    fn state(&self) -> isize {
        self.data().state()
    }

    fn set_state(&mut self, state: isize) {
        self.data_mut().set_state(state);
    }

    /// Adds a listener for syntax and prediction diagnostics.
    ///
    /// Recognizers start with one [`ConsoleErrorListener`]. Call
    /// [`Self::remove_error_listeners`] before adding a replacement when
    /// diagnostics should not also be written to stderr.
    fn add_error_listener<L>(&mut self, listener: L)
    where
        Self: Sized,
        L: for<'a> ErrorListener<dyn Recognizer + 'a> + Send + 'static,
    {
        self.data_mut().add_error_listener(listener);
    }

    /// Removes every error listener, including the default console listener.
    fn remove_error_listeners(&mut self) {
        self.data_mut().remove_error_listeners();
    }

    /// Sends one diagnostic to every registered error listener.
    fn notify_error_listeners(
        &self,
        line: usize,
        column: usize,
        message: &str,
        error: Option<&AntlrError>,
    ) where
        Self: Sized,
    {
        self.data()
            .notify_error_listeners(self, line, column, message, error);
    }

    fn sempred(&mut self, _rule_index: usize, _pred_index: usize) -> bool {
        true
    }

    fn action(&mut self, _rule_index: usize, _action_index: usize) {}
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
mod tests {
    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct RecordedError {
        grammar_file_name: String,
        line: usize,
        column: usize,
        message: String,
        error: Option<AntlrError>,
    }

    #[derive(Clone, Debug)]
    struct RecordingErrorListener {
        errors: Arc<Mutex<Vec<RecordedError>>>,
    }

    impl<R> ErrorListener<R> for RecordingErrorListener
    where
        R: Recognizer + ?Sized,
    {
        fn syntax_error(
            &mut self,
            recognizer: &R,
            line: usize,
            column: usize,
            message: &str,
            error: Option<&AntlrError>,
        ) {
            self.errors
                .lock()
                .expect("recorded errors lock")
                .push(RecordedError {
                    grammar_file_name: recognizer.grammar_file_name().to_owned(),
                    line,
                    column,
                    message: message.to_owned(),
                    error: error.cloned(),
                });
        }
    }

    #[derive(Clone, Debug)]
    struct TestRecognizer {
        data: RecognizerData,
    }

    impl Recognizer for TestRecognizer {
        fn data(&self) -> &RecognizerData {
            &self.data
        }

        fn data_mut(&mut self) -> &mut RecognizerData {
            &mut self.data
        }
    }

    fn test_recognizer() -> TestRecognizer {
        TestRecognizer {
            data: RecognizerData::new(
                "Test.g4",
                Vocabulary::new(
                    std::iter::empty::<Option<&str>>(),
                    std::iter::empty::<Option<&str>>(),
                    std::iter::empty::<Option<&str>>(),
                ),
            ),
        }
    }

    #[test]
    fn recognizers_replace_the_default_console_error_listener() {
        let mut recognizer = test_recognizer();
        assert_eq!(recognizer.data.error_listeners.len(), 1);

        recognizer.remove_error_listeners();
        assert!(recognizer.data.error_listeners.is_empty());

        let errors = Arc::new(Mutex::new(Vec::new()));
        recognizer.add_error_listener(RecordingErrorListener {
            errors: Arc::clone(&errors),
        });
        let error = AntlrError::ParserError {
            line: 3,
            column: 5,
            message: "unexpected token".to_owned(),
        };
        recognizer.notify_error_listeners(3, 5, "unexpected token", Some(&error));

        insta::assert_debug_snapshot!(
            "recognizers_replace_the_default_console_error_listener",
            *errors.lock().expect("recorded errors lock")
        );
    }

    #[test]
    fn recognizer_data_remains_send_and_sync() {
        fn assert_send_and_sync<T: Send + Sync>() {}

        assert_send_and_sync::<RecognizerData>();
    }

    #[test]
    fn cloned_recognizers_can_reconfigure_their_listener_lists_independently() {
        let mut original = test_recognizer();
        let clone = original.clone();

        original.remove_error_listeners();

        assert!(original.data.error_listeners.is_empty());
        assert_eq!(clone.data.error_listeners.len(), 1);
    }
}
