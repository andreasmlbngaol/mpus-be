/// Input validation for user-supplied fields. Pure functions, easy to test.

pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

pub fn email(value: &str) -> bool {
    let Some(at) = value.find('@') else {
        return false;
    };
    if at == 0 {
        return false;
    }
    let domain = &value[at + 1..];
    domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

pub fn username(value: &str) -> bool {
    let len = value.chars().count();
    (3..=30).contains(&len)
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

pub fn nickname(value: &str) -> bool {
    (1..=40).contains(&value.trim().chars().count())
}

pub fn password(value: &str) -> bool {
    value.chars().count() >= 8
}

/// A name a user gives a cat: 1-40 chars, no control characters.
pub fn cat_name(value: &str) -> bool {
    let t = value.trim();
    (1..=40).contains(&t.chars().count()) && !t.chars().any(char::is_control)
}

/// A cat review: 1-500 chars, no control characters (newlines allowed).
pub fn cat_review(value: &str) -> bool {
    let t = value.trim();
    (1..=500).contains(&t.chars().count())
        && !t.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

/// An optional note attached to a report: 1-200 chars, no control characters.
pub fn report_reason(value: &str) -> bool {
    let t = value.trim();
    (1..=200).contains(&t.chars().count())
        && !t.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_cases() {
        assert!(email("a@b.co"));
        assert!(email("user.name@sub.domain.com"));
        assert!(!email("noatsign"));
        assert!(!email("@b.co"));
        assert!(!email("a@b"));
        assert!(!email("a@.co"));
        assert!(!email("a@b."));
    }

    #[test]
    fn username_cases() {
        assert!(username("abc"));
        assert!(username("user_name.01"));
        assert!(!username("ab"));
        assert!(!username(&"x".repeat(31)));
        assert!(!username("has space"));
        assert!(!username("café")); // non-ASCII is rejected
    }

    #[test]
    fn nickname_cases() {
        assert!(nickname("A"));
        assert!(nickname(&"x".repeat(40)));
        assert!(!nickname(""));
        assert!(!nickname("   "));
        assert!(!nickname(&"x".repeat(41)));
    }

    #[test]
    fn password_cases() {
        assert!(password("12345678"));
        assert!(!password("1234567"));
    }

    #[test]
    fn cat_name_cases() {
        assert!(cat_name("Milo"));
        assert!(cat_name("Si Oren"));
        assert!(!cat_name(""));
        assert!(!cat_name("   "));
        assert!(!cat_name(&"x".repeat(41)));
        assert!(!cat_name("bad\u{0}name"));
    }

    #[test]
    fn normalization() {
        assert_eq!(normalize_email("  User@Example.COM "), "user@example.com");
    }

    #[test]
    fn report_reason_cases() {
        assert!(report_reason("spam"));
        assert!(!report_reason(""));
        assert!(!report_reason("   "));
        assert!(!report_reason(&"x".repeat(201)));
    }
}
