# The sync relay (crates/astra-server) on Cloud Run.
# Long-lived WebSockets: 60-minute request timeout (clients reconnect with backoff), always-allocated CPU so
# keepalive pings run, session affinity, and ingress only through the load balancer.

resource "google_service_account" "relay" {
  account_id   = "${var.name}-relay"
  display_name = "Astra relay (Cloud Run runtime)"
}

resource "google_secret_manager_secret_iam_member" "relay" {
  for_each  = google_secret_manager_secret.this
  secret_id = each.value.id
  role      = "roles/secretmanager.secretAccessor"
  member    = google_service_account.relay.member
}

resource "google_secret_manager_secret_iam_member" "relay_github" {
  count     = local.github_redirect_flow ? 1 : 0
  secret_id = google_secret_manager_secret.github_client_secret[0].id
  role      = "roles/secretmanager.secretAccessor"
  member    = google_service_account.relay.member
}

resource "google_project_iam_member" "relay" {
  for_each = toset(["roles/logging.logWriter", "roles/monitoring.metricWriter", "roles/cloudtrace.agent"])
  project  = var.project_id
  role     = each.value
  member   = google_service_account.relay.member
}

resource "google_cloud_run_v2_service" "relay" {
  name                 = "${var.name}-relay"
  location             = var.region
  ingress              = "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER"
  deletion_protection  = var.deletion_protection
  invoker_iam_disabled = true # public endpoint; the relay authenticates every connection with its own JWT
  labels               = local.labels

  template {
    service_account                  = google_service_account.relay.email
    timeout                          = "3600s"
    session_affinity                 = true
    max_instance_request_concurrency = 1000
    execution_environment            = "EXECUTION_ENVIRONMENT_GEN2"

    scaling {
      min_instance_count = var.relay_min_instances
      max_instance_count = var.relay_max_instances
    }

    vpc_access {
      egress = "PRIVATE_RANGES_ONLY"
      network_interfaces {
        network    = google_compute_network.this.id
        subnetwork = google_compute_subnetwork.run.id
      }
    }

    containers {
      image = var.relay_image

      ports {
        container_port = 8080
      }

      resources {
        limits            = { cpu = var.relay_cpu, memory = var.relay_memory }
        cpu_idle          = false
        startup_cpu_boost = true
      }

      env {
        name = "DATABASE_URL"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.this["database-url"].secret_id
            version = "latest"
          }
        }
      }

      env {
        name = "ASTRA_JWT_SECRET"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.this["jwt-secret"].secret_id
            version = "latest"
          }
        }
      }

      env {
        name  = "ASTRA_ALLOWED_ORIGINS"
        value = "https://${var.web_domain}"
      }

      # GitHub sends sign-ins back to <this>/v1/auth/redirect/callback.
      env {
        name  = "ASTRA_PUBLIC_URL"
        value = "https://${var.sync_domain}"
      }

      dynamic "env" {
        for_each = local.github_redirect_flow ? ["GITHUB_CLIENT_SECRET"] : []
        content {
          name = env.value
          value_source {
            secret_key_ref {
              secret  = google_secret_manager_secret.github_client_secret[0].secret_id
              version = "latest"
            }
          }
        }
      }

      env {
        name  = "ASTRA_LOG_FORMAT"
        value = "json"
      }

      env {
        name  = "RUST_LOG"
        value = "info"
      }

      dynamic "env" {
        for_each = { for k, v in {
          GITHUB_CLIENT_ID = var.github_oauth_client_id
          GITLAB_CLIENT_ID = var.gitlab_oauth_client_id
        } : k => v if v != "" }
        content {
          name  = env.key
          value = env.value
        }
      }

      startup_probe {
        initial_delay_seconds = 2
        period_seconds        = 3
        timeout_seconds       = 2
        failure_threshold     = 20
        http_get {
          path = "/readyz"
        }
      }

      liveness_probe {
        period_seconds    = 15
        timeout_seconds   = 3
        failure_threshold = 3
        http_get {
          path = "/healthz"
        }
      }
    }
  }

  lifecycle {
    # CI deploys new images (gcloud run deploy); Terraform owns everything else.
    ignore_changes = [
      template[0].containers[0].image,
      client,
      client_version,
    ]
  }

  depends_on = [
    google_secret_manager_secret_iam_member.relay,
    google_secret_manager_secret_iam_member.relay_github,
    google_secret_manager_secret_version.this,
    google_secret_manager_secret_version.github_client_secret,
  ]
}
