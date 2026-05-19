/// A simple user record used as a fixture for parser/skeleton tests.
pub struct User {
    pub name: String,
    pub age: u32,
}

impl User {
    /// Construct a new User, validating that the age is realistic.
    pub fn new(name: &str, age: u32) -> Self {
        let u = User {
            name: name.to_string(),
            age,
        };
        assert!(u.validate(), "age must be less than 150");
        u
    }

    /// Private validation — not visible in Minimal skeleton output.
    fn validate(&self) -> bool {
        self.age < 150
    }
}

/// Anything that can describe itself in a human-readable sentence.
pub trait Describable {
    fn describe(&self) -> String;
}

impl Describable for User {
    fn describe(&self) -> String {
        format!("{} (age {})", self.name, self.age)
    }
}
