//! Connect command implementation.

use crate::cli::ConnectArgs;
use crate::config::{Config, Profile};
use crate::error::Result;
use crate::output::Formatter;
use boswell_sdk::BoswellClient;

/// Execute the connect command.
pub async fn execute_connect(
    args: ConnectArgs,
    config: &mut Config,
    formatter: &Formatter,
) -> Result<BoswellClient> {
    // Determine connection parameters
    let url = if let Some(url) = args.url {
        url
    } else {
        // Use active profile
        let profile = config.get_active_profile()?;
        profile.router_url.clone()
    };

    // Create client
    let mut client = BoswellClient::new(&url);
    
    // Connect to router (auto-selects instance)
    client.connect().await?;

    println!("{}", formatter.connection_info(&url, "auto"));

    // Save as profile if requested
    if let Some(profile_name) = args.save_as {
        let profile = Profile {
            router_url: url.clone(),
            instance_id: "auto".to_string(),
            namespace: args.namespace,
        };
        config.set_profile(profile_name.clone(), profile);
        config.save()?;
        println!(
            "{}",
            formatter.success(&format!("Profile '{}' saved", profile_name))
        );
    }

    Ok(client)
}
