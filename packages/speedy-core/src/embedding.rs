pub struct EmbeddingInput {
    pub text: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Embedding {
    pub vector: Vec<f32>,
    pub dimension: usize,
}

impl Embedding {
    pub fn new(vector: Vec<f32>) -> Self {
        let dimension = vector.len();
        Self { vector, dimension }
    }

    pub fn cosine_similarity(&self, other: &Embedding) -> f64 {
        let dot: f32 = self
            .vector
            .iter()
            .zip(&other.vector)
            .map(|(a, b)| a * b)
            .sum();
        let norm_a: f32 = self.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }
        (dot / (norm_a * norm_b)) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = Embedding::new(vec![1.0, 0.0, 0.0]);
        let b = Embedding::new(vec![1.0, 0.0, 0.0]);
        let sim = a.cosine_similarity(&b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = Embedding::new(vec![1.0, 0.0]);
        let b = Embedding::new(vec![0.0, 1.0]);
        let sim = a.cosine_similarity(&b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = Embedding::new(vec![1.0, 0.0]);
        let b = Embedding::new(vec![-1.0, 0.0]);
        let sim = a.cosine_similarity(&b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = Embedding::new(vec![0.0, 0.0]);
        let b = Embedding::new(vec![1.0, 0.0]);
        assert_eq!(a.cosine_similarity(&b), 0.0);
    }

    #[test]
    fn test_embedding_dimension() {
        let e = Embedding::new(vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(e.dimension, 4);
    }

    #[test]
    fn test_cosine_similarity_different_dimensions() {
        let a = Embedding::new(vec![1.0, 0.0, 0.0]);
        let b = Embedding::new(vec![1.0, 0.0]);
        let sim = a.cosine_similarity(&b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_small_values() {
        let a = Embedding::new(vec![1e-8, 0.0]);
        let b = Embedding::new(vec![0.0, 1e-8]);
        let sim = a.cosine_similarity(&b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_new_sets_dimension() {
        let e = Embedding::new(vec![]);
        assert_eq!(e.dimension, 0);
        assert!(e.vector.is_empty());
    }
}
