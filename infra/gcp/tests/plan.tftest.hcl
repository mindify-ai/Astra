# Offline plan checks with mocked providers: `terraform test` (no GCP credentials needed).

mock_provider "google" {
  mock_data "google_project" {
    defaults = { number = "123456789012" }
  }
  mock_resource "google_sql_database_instance" {
    defaults = { private_ip_address = "10.20.0.3", connection_name = "p:asia-east1:astra-pg" }
  }
  mock_resource "google_iam_workload_identity_pool" {
    defaults = { name = "projects/123456789012/locations/global/workloadIdentityPools/astra-github" }
  }
  mock_resource "google_compute_global_address" {
    defaults = { address = "203.0.113.10" }
  }
}

mock_provider "random" {}

variables {
  project_id  = "astra-test"
  web_domain  = "app.example.com"
  sync_domain = "sync.example.com"
}

run "secure_defaults" {
  command = plan

  assert {
    condition     = google_cloud_run_v2_service.relay.ingress == "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER"
    error_message = "The relay must only be reachable through the load balancer (Cloud Armor in front)."
  }
  assert {
    condition     = google_cloud_run_v2_service.relay.template[0].scaling[0].max_instance_count == 1
    error_message = "Relay fan-out is in-process: one instance until NATS ships."
  }
  assert {
    condition     = google_cloud_run_v2_service.relay.template[0].timeout == "3600s"
    error_message = "WebSockets need the 60-minute request timeout."
  }
  assert {
    condition     = google_cloud_run_v2_service.relay.template[0].containers[0].resources[0].cpu_idle == false
    error_message = "CPU must stay allocated so keepalive pings run between requests."
  }
  assert {
    condition     = google_sql_database_instance.this.settings[0].ip_configuration[0].ipv4_enabled == false
    error_message = "Cloud SQL must have no public IP."
  }
  assert {
    condition     = google_sql_database_instance.this.settings[0].ip_configuration[0].ssl_mode == "ENCRYPTED_ONLY"
    error_message = "Database connections must use TLS."
  }
  assert {
    condition     = google_sql_database_instance.this.deletion_protection
    error_message = "Production database must be protected from destroy."
  }
  assert {
    condition     = strcontains(google_iam_workload_identity_pool_provider.github.attribute_condition, "assertion.repository == 'mindify-ai/Astra'")
    error_message = "Only the Astra repository may deploy."
  }
  assert {
    condition     = strcontains(google_iam_workload_identity_pool_provider.github.attribute_condition, "refs/heads/main")
    error_message = "Only main may deploy by default."
  }
  assert {
    condition     = toset(google_compute_managed_ssl_certificate.this.managed[0].domains) == toset(["app.example.com", "sync.example.com"])
    error_message = "One certificate covers both domains."
  }
  assert {
    condition     = length(google_dns_record_set.this) == 0
    error_message = "No DNS records unless a zone is given."
  }
  assert {
    condition     = length(google_secret_manager_secret.github_client_secret) == 0
    error_message = "No GitHub client secret unless one is given (the device flow only)."
  }
  assert {
    condition     = length([for e in google_cloud_run_v2_service.relay.template[0].containers[0].env : e if e.name == "ASTRA_PUBLIC_URL" && e.value == "https://sync.example.com"]) == 1
    error_message = "The relay must know its public URL for the GitHub callback."
  }
}

run "github_one_click_sign_in" {
  command = plan

  variables {
    github_oauth_client_id     = "Ov23liExample"
    github_oauth_client_secret = "not-a-real-secret"
  }

  assert {
    condition     = google_secret_manager_secret.github_client_secret[0].secret_id == "astra-github-client-secret"
    error_message = "The client secret is kept in Secret Manager."
  }
  assert {
    condition     = length([for e in google_cloud_run_v2_service.relay.template[0].containers[0].env : e if e.name == "GITHUB_CLIENT_SECRET" && length(e.value_source) == 1]) == 1
    error_message = "The relay reads the client secret from Secret Manager, never from a plain env value."
  }
  assert {
    condition     = length([for e in google_cloud_run_v2_service.relay.template[0].containers[0].env : e if e.name == "GITHUB_CLIENT_SECRET" && e.value != null && e.value != ""]) == 0
    error_message = "The client secret must not appear as a plain env value."
  }
  assert {
    condition     = length(google_secret_manager_secret_iam_member.relay_github) == 1
    error_message = "The relay's service account can read the client secret."
  }
}

run "dns_and_scale_out" {
  command = plan

  variables {
    dns_zone_name        = "example-com"
    relay_max_instances  = 4
    db_high_availability = true
    github_deploy_ref    = ""
  }

  override_data {
    target = data.google_dns_managed_zone.this[0]
    values = { name = "example-com" }
  }

  assert {
    condition     = length(google_dns_record_set.this) == 2
    error_message = "Both domains get A records."
  }
  assert {
    condition     = google_sql_database_instance.this.settings[0].availability_type == "REGIONAL"
    error_message = "HA switches the database to regional."
  }
  assert {
    condition     = !strcontains(google_iam_workload_identity_pool_provider.github.attribute_condition, "assertion.ref")
    error_message = "An empty deploy ref allows any ref."
  }
}

run "rejects_bad_name" {
  command = plan
  variables {
    name = "Astra_Prod"
  }
  expect_failures = [var.name]
}
