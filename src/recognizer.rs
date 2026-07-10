use crate::vocabulary::Vocabulary;

#[derive(Clone, Debug)]
pub struct RecognizerData {
    grammar_file_name: String,
    rule_names: Vec<String>,
    channel_names: Vec<String>,
    mode_names: Vec<String>,
    vocabulary: Vocabulary,
    state: isize,
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

    fn sempred(&mut self, _rule_index: usize, _pred_index: usize) -> bool {
        true
    }

    fn action(&mut self, _rule_index: usize, _action_index: usize) {}
}
