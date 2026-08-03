pub const EOF: i32 = -1;
pub const UNKNOWN_SOURCE_NAME: &str = "<unknown>";

pub trait IntStream {
    fn consume(&mut self);
    fn la(&mut self, offset: isize) -> i32;
    fn mark(&mut self) -> isize {
        -1
    }
    fn release(&mut self, _marker: isize) {}
    fn index(&self) -> usize;
    fn seek(&mut self, index: usize);
    fn size(&self) -> usize;
    fn source_name(&self) -> &str {
        UNKNOWN_SOURCE_NAME
    }
}
