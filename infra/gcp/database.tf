# Postgres for the relay's op log, workspaces, memberships and node registry.

resource "random_id" "db_suffix" {
  byte_length = 3
}

resource "google_sql_database_instance" "this" {
  # Instance names cannot be reused for a week after deletion, hence the suffix.
  name                = "${var.name}-pg-${random_id.db_suffix.hex}"
  region              = var.region
  database_version    = "POSTGRES_17"
  deletion_protection = var.deletion_protection

  settings {
    tier                        = var.db_tier
    edition                     = "ENTERPRISE"
    availability_type           = var.db_high_availability ? "REGIONAL" : "ZONAL"
    disk_type                   = "PD_SSD"
    disk_size                   = var.db_disk_gb
    disk_autoresize             = true
    deletion_protection_enabled = var.deletion_protection
    user_labels                 = local.labels

    ip_configuration {
      ipv4_enabled    = false
      private_network = google_compute_network.this.id
      ssl_mode        = "ENCRYPTED_ONLY"
    }

    backup_configuration {
      enabled                        = true
      point_in_time_recovery_enabled = true
      start_time                     = "18:00"
      transaction_log_retention_days = 7
      backup_retention_settings {
        retained_backups = 14
      }
    }

    maintenance_window {
      day          = 7
      hour         = 19
      update_track = "stable"
    }

    insights_config {
      query_insights_enabled  = true
      record_application_tags = false
      record_client_address   = false
    }

    database_flags {
      name  = "max_connections"
      value = "200"
    }
  }

  depends_on = [google_service_networking_connection.this]
}

resource "google_sql_database" "astra" {
  name     = "astra"
  instance = google_sql_database_instance.this.name
}

resource "random_password" "db" {
  length  = 40
  special = false # keeps the connection URL free of characters that need escaping
}

resource "google_sql_user" "astra" {
  name     = "astra"
  instance = google_sql_database_instance.this.name
  password = random_password.db.result
}
