# Auto-push

Auto-push is configured under **Settings → Automation** and is enabled by
default.

When enabled, honey automatically delivers desired configuration after
resource changes and from background operations such as quota enforcement,
scheduled operations and CDN rotation. The reconcile loop also repairs drift
after a node reconnects.

When disabled, changes remain in the database until an operator uses **Push**
on the node. Connectivity and heartbeats continue, and manual push always
bypasses the Auto-push switch.

The setting is stored as `auto_push_enabled` in `app_settings` and is applied
live without restarting master or agent services.
