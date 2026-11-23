# Optional: A records in an existing Cloud DNS zone. Otherwise point both domains at output `load_balancer_ip`.

data "google_dns_managed_zone" "this" {
  count = var.dns_zone_name == "" ? 0 : 1
  name  = var.dns_zone_name
}

resource "google_dns_record_set" "this" {
  for_each     = var.dns_zone_name == "" ? toset([]) : toset([var.web_domain, var.sync_domain])
  managed_zone = data.google_dns_managed_zone.this[0].name
  name         = "${each.value}."
  type         = "A"
  ttl          = 300
  rrdatas      = [google_compute_global_address.lb.address]
}
