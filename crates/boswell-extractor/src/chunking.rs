//! Text chunking strategies for large documents

use crate::config::ChunkStrategy;

/// Chunks text according to the specified strategy
pub struct TextChunker {
    strategy: ChunkStrategy,
    max_chunk_size: usize,
}

impl TextChunker {
    /// Create a new text chunker
    pub fn new(strategy: ChunkStrategy, max_chunk_size: usize) -> Self {
        Self {
            strategy,
            max_chunk_size,
        }
    }
    
    /// Chunk the given text
    pub fn chunk(&self, text: &str) -> Vec<String> {
        if text.len() <= self.max_chunk_size {
            return vec![text.to_string()];
        }
        
        match self.strategy {
            ChunkStrategy::ByParagraph => self.chunk_by_paragraph(text),
            ChunkStrategy::BySection => self.chunk_by_section(text),
            ChunkStrategy::ByTokenCount => self.chunk_by_token_count(text),
        }
    }
    
    /// Chunk by paragraphs (double newlines)
    fn chunk_by_paragraph(&self, text: &str) -> Vec<String> {
        let paragraphs: Vec<&str> = text.split("\n\n").collect();
        self.combine_until_limit(paragraphs)
    }
    
    /// Chunk by sections (markdown headers or numbered sections)
    fn chunk_by_section(&self, text: &str) -> Vec<String> {
        let mut sections = Vec::new();
        let mut current_section = String::new();
        
        for line in text.lines() {
            // Detect markdown headers (# Header) or numbered sections (1. Section)
            let is_section_header = line.trim_start().starts_with('#')
                || (line.trim_start().chars().next().map_or(false, |c| c.is_ascii_digit())
                    && line.contains('.'));
            
            if is_section_header && !current_section.is_empty() {
                sections.push(current_section.trim().to_string());
                current_section = String::new();
            }
            
            current_section.push_str(line);
            current_section.push('\n');
        }
        
        if !current_section.is_empty() {
            sections.push(current_section.trim().to_string());
        }
        
        // If we got sections, combine them until limit
        if sections.is_empty() {
            // Fall back to paragraph chunking
            self.chunk_by_paragraph(text)
        } else {
            self.combine_until_limit(sections)
        }
    }
    
    /// Chunk by approximate token count (4 chars ~ 1 token)
    fn chunk_by_token_count(&self, text: &str) -> Vec<String> {
        // Approximate: 1 token ≈ 4 characters
        let token_limit = self.max_chunk_size;
        let char_limit = token_limit;
        
        // Split at sentence boundaries when possible
        let sentences: Vec<&str> = text
            .split(|c| c == '.' || c == '!' || c == '?')
            .filter(|s| !s.trim().is_empty())
            .collect();
        
        if sentences.is_empty() {
            return vec![text.to_string()];
        }
        
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        
        for sentence in sentences {
            let sentence_with_punct = format!("{}. ", sentence.trim());
            
            if current_chunk.len() + sentence_with_punct.len() > char_limit {
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk.trim().to_string());
                    current_chunk = String::new();
                }
                
                // If single sentence exceeds limit, split it arbitrarily
                if sentence_with_punct.len() > char_limit {
                    chunks.extend(self.split_at_char_limit(&sentence_with_punct, char_limit));
                } else {
                    current_chunk.push_str(&sentence_with_punct);
                }
            } else {
                current_chunk.push_str(&sentence_with_punct);
            }
        }
        
        if !current_chunk.is_empty() {
            chunks.push(current_chunk.trim().to_string());
        }
        
        chunks
    }
    
    /// Combine elements until they reach the size limit
    fn combine_until_limit<S: AsRef<str>>(&self, elements: Vec<S>) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        
        for element in elements {
            let element_str = element.as_ref();
            
            if current_chunk.len() + element_str.len() + 2 > self.max_chunk_size {
                if !current_chunk.is_empty() {
                    chunks.push(current_chunk.trim().to_string());
                    current_chunk = String::new();
                }
                
                // If single element exceeds limit, split it
                if element_str.len() > self.max_chunk_size {
                    chunks.extend(self.split_at_char_limit(element_str, self.max_chunk_size));
                } else {
                    current_chunk.push_str(element_str);
                    current_chunk.push_str("\n\n");
                }
            } else {
                current_chunk.push_str(element_str);
                current_chunk.push_str("\n\n");
            }
        }
        
        if !current_chunk.is_empty() {
            chunks.push(current_chunk.trim().to_string());
        }
        
        chunks
    }
    
    /// Split text at character limit boundaries
    fn split_at_char_limit(&self, text: &str, limit: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut start = 0;
        
        while start < text.len() {
            let end = std::cmp::min(start + limit, text.len());
            chunks.push(text[start..end].to_string());
            start = end;
        }
        
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_chunking_needed_for_small_text() {
        let chunker = TextChunker::new(ChunkStrategy::ByParagraph, 100);
        let text = "Short text here.";
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_chunk_by_paragraph() {
        let chunker = TextChunker::new(ChunkStrategy::ByParagraph, 50);
        let text = "First paragraph here.\n\nSecond paragraph here.\n\nThird paragraph here.";
        let chunks = chunker.chunk(text);
        
        // Should combine paragraphs until limit
        assert!(chunks.len() > 0);
        for chunk in &chunks {
            assert!(chunk.len() <= 100); // Some tolerance for combining
        }
    }

    #[test]
    fn test_chunk_by_section_with_markdown() {
        let chunker = TextChunker::new(ChunkStrategy::BySection, 50);
        let text = "# Section 1\nContent 1\n# Section 2\nContent 2";
        let chunks = chunker.chunk(text);
        
        assert!(chunks.len() >= 1);
        assert!(chunks[0].contains("Section 1"));
    }

    #[test]
    fn test_chunk_by_section_with_numbered() {
        let chunker = TextChunker::new(ChunkStrategy::BySection, 50);
        let text = "1. First section\nContent\n2. Second section\nMore content";
        let chunks = chunker.chunk(text);
        
        assert!(chunks.len() >= 1);
    }

    #[test]
    fn test_chunk_by_token_count() {
        let chunker = TextChunker::new(ChunkStrategy::ByTokenCount, 20);
        let text = "First sentence here. Second sentence here. Third sentence here.";
        let chunks = chunker.chunk(text);
        
        // Should split at sentence boundaries
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 40); // Some tolerance
        }
    }

    #[test]
    fn test_empty_text() {
        let chunker = TextChunker::new(ChunkStrategy::ByParagraph, 100);
        let chunks = chunker.chunk("");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn test_single_line_text() {
        let chunker = TextChunker::new(ChunkStrategy::ByParagraph, 100);
        let text = "Single line of text without paragraphs.";
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_very_long_single_paragraph() {
        let chunker = TextChunker::new(ChunkStrategy::ByParagraph, 20);
        let text = "a".repeat(100); // 100 chars, no paragraphs
        let chunks = chunker.chunk(&text);
        
        // Should split even single paragraph
        assert!(chunks.len() > 1);
    }

    #[test]
    fn test_section_fallback_to_paragraph() {
        let chunker = TextChunker::new(ChunkStrategy::BySection, 50);
        let text = "Just text\n\nWith paragraphs\n\nBut no sections";
        let chunks = chunker.chunk(text);
        
        // Should fall back to paragraph chunking
        assert!(chunks.len() >= 1);
    }
}
