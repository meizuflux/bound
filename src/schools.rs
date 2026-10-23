pub struct School {
    pub name: &'static str,
    pub email_suffixes: &'static [&'static str],
}

pub const SCHOOLS: &[School] = &[School {
    name: "Duke University",
    email_suffixes: &["duke.edu"],
}];

pub fn supported_names() -> String {
    SCHOOLS
        .iter()
        .map(|school| school.name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn from_email(email: &str) -> Option<&'static School> {
    let address: lettre::Address = email.trim().parse().ok()?;
    SCHOOLS.iter().find(|school| {
        school
            .email_suffixes
            .iter()
            .any(|suffix| address.domain().eq_ignore_ascii_case(suffix))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_supported_school_domains_exactly() {
        assert_eq!(
            from_email("student@DUKE.edu").unwrap().name,
            "Duke University"
        );
        for email in [
            "a@example.com",
            "a@sub.duke.edu",
            "a@duke.edu.evil.test",
            "a@@duke.edu",
        ] {
            assert!(from_email(email).is_none());
        }
    }
}
