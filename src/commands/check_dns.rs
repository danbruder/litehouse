use anyhow::Result;
use tracing::{info, instrument};

use crate::config::ServerConfig;
use crate::install::executor::run_command;

#[instrument]
pub async fn execute() -> Result<()> {
    info!("Checking DNS configuration");

    // Load server config to get domain
    let config = ServerConfig::load()?;
    let domain = config
        .domain
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No domain configured. Run 'lh install --domain <domain>' first."))?;

    println!("\nChecking DNS configuration for domain: {}", domain);
    println!("{}", "=".repeat(60));

    // Get this server's public IP
    println!("\n1. Determining server's public IP address...");
    let server_ip = match run_command("curl -4 -s ifconfig.me --max-time 10") {
        Ok(ip) => ip.trim().to_string(),
        Err(_) => {
            // Try alternative service
            match run_command("curl -4 -s icanhazip.com --max-time 10") {
                Ok(ip) => ip.trim().to_string(),
                Err(e) => {
                    println!("   ✗ Failed to determine server's public IP: {}", e);
                    anyhow::bail!("Could not determine server's public IP address");
                }
            }
        }
    };

    println!("   ✓ Server public IP: {}", server_ip);

    // Check if dig is available, otherwise use host
    let dns_available = run_command("which dig").is_ok() || run_command("which host").is_ok();
    if !dns_available {
        println!("\n   ✗ Neither 'dig' nor 'host' command is available");
        println!("     Please install dnsutils (Debian/Ubuntu) or bind-utils (RHEL/CentOS)");
        anyhow::bail!("DNS tools not available");
    }

    // Check admin subdomain
    let admin_label = config.admin_label();
    println!("\n2. Checking DNS for {}.{}...", admin_label, domain);
    check_dns_record(&format!("{}.{}", admin_label, domain), &server_ip)?;

    // Check wildcard domain (test with a random subdomain)
    println!("\n3. Checking DNS for *.{}...", domain);
    let test_subdomain = format!("test-app.{}", domain);
    check_dns_record(&test_subdomain, &server_ip)?;

    println!("\n{}", "=".repeat(60));
    println!("✓ DNS configuration is correct!");
    println!("\nYour litehouse server should be accessible at:");
    println!("  - Admin: https://{}.{}", admin_label, domain);
    println!("  - Apps:  https://<app-name>.{}", domain);
    println!("{}", "=".repeat(60));

    Ok(())
}

fn check_dns_record(hostname: &str, expected_ip: &str) -> Result<()> {
    let dns_command = if run_command("which dig").is_ok() {
        format!("dig +short {} A | head -1", hostname)
    } else {
        format!("host -t A {} | grep 'has address' | awk '{{print $NF}}' | head -1", hostname)
    };

    match run_command(&dns_command) {
        Ok(resolved_ip) => {
            let resolved_ip = resolved_ip.trim();
            if resolved_ip.is_empty() {
                println!("   ✗ {} does not resolve to any IP", hostname);
                println!("\n   Required DNS configuration:");
                println!("   Please add this A record in your DNS provider:");
                println!("     {} -> {}", hostname, expected_ip);
                anyhow::bail!("DNS record not found for {}", hostname);
            } else if resolved_ip != expected_ip {
                println!("   ✗ {} resolves to {} (expected: {})", hostname, resolved_ip, expected_ip);
                println!("\n   Required DNS configuration:");
                println!("   Please update this A record in your DNS provider:");
                println!("     {} -> {}", hostname, expected_ip);
                anyhow::bail!("DNS record incorrect for {}", hostname);
            } else {
                println!("   ✓ {} -> {}", hostname, resolved_ip);
            }
        }
        Err(e) => {
            println!("   ✗ Failed to resolve {}: {}", hostname, e);
            println!("\n   Required DNS configuration:");
            println!("   Please add this A record in your DNS provider:");
            println!("     {} -> {}", hostname, expected_ip);
            anyhow::bail!("Failed to resolve {}", hostname);
        }
    }

    Ok(())
}
