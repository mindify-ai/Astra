# Server secrets. Values also land in Terraform state: keep the state bucket private (see README).

resource "random_password" "jwt" {
  length  = 64
  special = false
}

locals {
  database_url = format(
    "postgres://%s:%s@%s:5432/%s?sslmode=require",
    google_sql_user.astra.name,
    random_password.db.result,
    google_sql_database_instance.this.private_ip_address,
    google_sql_database.astra.name,
  )

  secrets = {
    "database-url" = local.database_url
    "jwt-secret"   = random_password.jwt.result
  }
}

resource "google_secret_manager_secret" "this" {
  for_each  = local.secrets
  secret_id = "${var.name}-${each.key}"
  labels    = local.labels

  replication {
    auto {}
  }

  depends_on = [google_project_service.this]
}

resource "google_secret_manager_secret_version" "this" {
  for_each    = local.secrets
  secret      = google_secret_manager_secret.this[each.key].id
  secret_data = each.value
}

# The GitHub OAuth app's client secret, only when one is given (it turns on one-click sign-in). Separate from
# `local.secrets` because a sensitive value can't decide `for_each` keys; whether it is set is not secret.
locals {
  github_redirect_flow = nonsensitive(var.github_oauth_client_secret != "")
}

resource "google_secret_manager_secret" "github_client_secret" {
  count     = local.github_redirect_flow ? 1 : 0
  secret_id = "${var.name}-github-client-secret"
  labels    = local.labels

  replication {
    auto {}
  }

  depends_on = [google_project_service.this]
}

resource "google_secret_manager_secret_version" "github_client_secret" {
  count       = local.github_redirect_flow ? 1 : 0
  secret      = google_secret_manager_secret.github_client_secret[0].id
  secret_data = var.github_oauth_client_secret
}
