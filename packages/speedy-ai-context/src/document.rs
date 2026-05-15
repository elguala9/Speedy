pub struct Chunk {
    pub text: String,
    pub line: usize,
}

pub struct Document {
    pub id: String,
    pub title: String,
    pub content: String,
}

impl Document {
    pub fn new(id: &str, title: &str, content: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
        }
    }

    pub fn chunk_file(content: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
        if content.len() <= chunk_size {
            return vec![Chunk {
                text: content.to_string(),
                line: 1,
            }];
        }

        let separators = ["\n\n", "\n", ". ", " ", ""];
        let mut chunks = Vec::new();
        let mut start = 0;

        while start < content.len() {
            let end_target = (start + chunk_size).min(content.len());

            let split_pos = if end_target >= content.len() {
                content.len()
            } else {
                let search_slice = &content[start..end_target];
                let mut best = end_target;
                for sep in &separators {
                    if sep.is_empty() {
                        continue;
                    }
                    if let Some(pos) = search_slice.rfind(sep) {
                        best = start + pos + sep.len();
                        break;
                    }
                }
                best
            };

            let chunk_text = &content[start..split_pos];
            let line_num = content[..start].matches('\n').count() + 1;

            chunks.push(Chunk {
                text: chunk_text.to_string(),
                line: line_num,
            });

            if split_pos >= content.len() {
                break;
            }

            start = if split_pos > overlap {
                split_pos - overlap
            } else {
                split_pos
            };
        }

        chunks
    }

    pub fn split_into_chunks(&self, chunk_size: usize) -> Vec<String> {
        Self::chunk_file(&self.content, chunk_size, 200)
            .into_iter()
            .map(|c| c.text)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_small_text() {
        let chunks = Document::chunk_file("hello world", 1000, 200);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world");
        assert_eq!(chunks[0].line, 1);
    }

    #[test]
    fn test_chunk_splits_at_newline() {
        let text = "line one\nline two\nline three\nline four";
        let chunks = Document::chunk_file(text, 15, 0);
        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].line, 1);
    }

    #[test]
    fn test_chunk_line_numbers() {
        let text = "a\nb\nc\nd\ne\nf\ng\nh\ni\nl";
        let chunks = Document::chunk_file(text, 3, 0);
        assert!(chunks.len() > 5, "expected many small chunks");
        assert_eq!(chunks[0].line, 1);
        assert_eq!(chunks[1].line, 2);
        assert_eq!(chunks[2].line, 3);
    }

    #[test]
    fn test_split_into_chunks() {
        let doc = Document::new("t", "test", "hello world\nfoo bar\nbaz qux");
        let chunks = doc.split_into_chunks(100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world\nfoo bar\nbaz qux");
    }

    #[test]
    fn test_chunk_empty_content() {
        let chunks = Document::chunk_file("", 100, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "");
    }

    #[test]
    fn test_chunk_exact_size() {
        let text = "hello world";
        let chunks = Document::chunk_file(text, 11, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world");
    }

    #[test]
    fn test_chunk_overlap_produces_overlapping_text() {
        let text = "abc\ndef\nghi\njkl\nmno\npqr";
        let chunks = Document::chunk_file(text, 8, 4);
        assert!(chunks.len() >= 2);
        if chunks.len() >= 2 {
            assert!(chunks[0].text.len() >= 4);
            assert!(chunks[1].text.len() >= 4);
        }
    }

    #[test]
    fn test_chunk_line_numbers_multi_line() {
        let text = "a\nb\nc\nd\ne\nf\ng\nh\ni\nl\nm\nn";
        let chunks = Document::chunk_file(text, 3, 0);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.line, i + 1);
        }
    }

    #[test]
    fn test_document_new() {
        let doc = Document::new("id1", "title1", "content1");
        assert_eq!(doc.id, "id1");
        assert_eq!(doc.title, "title1");
        assert_eq!(doc.content, "content1");
    }

    #[test]
    fn test_split_into_chunks_multiple() {
        let long = (0..20).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let doc = Document::new("t", "test", &long);
        let chunks = doc.split_into_chunks(50);
        assert!(chunks.len() > 1);
        let total_len: usize = chunks.iter().map(|c| c.len()).sum();
        assert!(total_len >= long.len());
    }

    #[test]
    fn test_chunk_separator_preference() {
        let text = "paragraph one\n\nparagraph two. sentence b\n\nparagraph three";
        let chunks = Document::chunk_file(text, 30, 0);
        assert!(chunks.len() >= 3);
    }
}
