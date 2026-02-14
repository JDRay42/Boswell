//! Profile command implementation.

use crate::cli::{ProfileAction, ProfileArgs};
use crate::config::{Config, Profile};
use crate::error::Result;
use crate::output::Formatter;

/// Execute the profile command.
pub async fn execute_profile(
    args: ProfileArgs,
    config: &mut Config,
    formatter: &Formatter,
) -> Result<()> {
    match args.action {
        ProfileAction::List => list_profiles(config, formatter),
        ProfileAction::Show => show_active_profile(config, formatter),
        ProfileAction::Switch { name } => switch_profile(config, name, formatter),
        ProfileAction::Set {
            name,
            url,
            instance,
            namespace,
        } => set_profile(config, name, url, instance, namespace, formatter),
        ProfileAction::Delete { name } => delete_profile(config, name, formatter),
    }
}

/// List all profiles.
fn list_profiles(config: &Config, formatter: &Formatter) -> Result<()> {
    if config.profiles.is_empty() {
        println!("{}", formatter.info("No profiles configured"));
        return Ok(());
    }

    println!("Available profiles:");
    for (name, profile) in &config.profiles {
        let marker = if name == &config.active_profile {
            "* "
        } else {
            "  "
        };
        println!(
            "{}{}",
            marker,
            if name == &config.active_profile {
                formatter.success(name)
            } else {
                name.clone()
            }
        );
        println!("    URL: {}", profile.router_url);
        println!("    Instance: {}", profile.instance_id);
        if let Some(ns) = &profile.namespace {
            println!("    Namespace: {}", ns);
        }
    }

    Ok(())
}

/// Show the active profile.
fn show_active_profile(config: &Config, formatter: &Formatter) -> Result<()> {
    let profile = config.get_active_profile()?;
    
    println!("Active profile: {}", formatter.success(&config.active_profile));
    println!("  URL: {}", profile.router_url);
    println!("  Instance: {}", profile.instance_id);
    if let Some(ns) = &profile.namespace {
        println!("  Namespace: {}", ns);
    }

    Ok(())
}

/// Switch to a different profile.
fn switch_profile(config: &mut Config, name: String, formatter: &Formatter) -> Result<()> {
    config.switch_profile(name.clone())?;
    config.save()?;
    println!(
        "{}",
        formatter.success(&format!("Switched to profile '{}'", name))
    );
    Ok(())
}

/// Create or update a profile.
fn set_profile(
    config: &mut Config,
    name: String,
    url: String,
    instance: String,
    namespace: Option<String>,
    formatter: &Formatter,
) -> Result<()> {
    let profile = Profile {
        router_url: url,
        instance_id: instance,
        namespace,
    };

    let action = if config.profiles.contains_key(&name) {
        "Updated"
    } else {
        "Created"
    };

    config.set_profile(name.clone(), profile);
    config.save()?;

    println!(
        "{}",
        formatter.success(&format!("{} profile '{}'", action, name))
    );

    Ok(())
}

/// Delete a profile.
fn delete_profile(config: &mut Config, name: String, formatter: &Formatter) -> Result<()> {
    if name == config.active_profile {
        return Err(crate::error::CliError::NotPermitted(
            "Cannot delete the active profile".to_string(),
        ));
    }

    if config.profiles.remove(&name).is_some() {
        config.save()?;
        println!(
            "{}",
            formatter.success(&format!("Deleted profile '{}'", name))
        );
    } else {
        println!(
            "{}",
            formatter.warning(&format!("Profile '{}' does not exist", name))
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputFormat;

    #[test]
    fn test_set_and_switch_profile() {
        let mut config = Config::default();
        let formatter = Formatter::new(OutputFormat::Table, false);

        // Set a new profile
        set_profile(
            &mut config,
            "test".to_string(),
            "http://localhost:8080".to_string(),
            "test-instance".to_string(),
            None,
            &formatter,
        )
        .unwrap();

        assert!(config.profiles.contains_key("test"));

        // Switch to it
        switch_profile(&mut config, "test".to_string(), &formatter).unwrap();
        assert_eq!(config.active_profile, "test");
    }

    #[test]
    fn test_delete_active_profile() {
        let mut config = Config::default();
        let formatter = Formatter::new(OutputFormat::Table, false);

        let result = delete_profile(&mut config, "default".to_string(), &formatter);
        assert!(result.is_err());
    }
}
