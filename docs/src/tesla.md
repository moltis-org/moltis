# Tesla

Moltis can keep a local, durable copy of your Tesla vehicle data using Tesla's
Fleet API. The connector is **read-only**: it never sends vehicle commands and
never wakes a sleeping car.

Data lands in `<data_dir>/connectors.db` alongside every other connector, so a
trusted agent can answer questions about your car without contacting Tesla.

## What you need first

Fleet API is not open to anonymous clients. Before Moltis can talk to it you
need your own Tesla developer application, and that requires a domain you
control:

1. Register an application at [developer.tesla.com](https://developer.tesla.com).
   Request the read-only scopes you want: `openid`, `offline_access`,
   `vehicle_device_data`, and `vehicle_location` if you want precise
   coordinates.
2. Generate a key pair and host the public key on your application's domain at
   `https://<your-domain>/.well-known/appspecific/com.tesla.3p.public-key.pem`.
   Keep the private key off the web.
3. Complete **partner registration** for the application. Fleet API rejects
   every call until this is done, and Moltis reports that specific failure
   rather than a generic error.
4. Complete the authorization-code flow once to obtain a **refresh token**.

Moltis deliberately does not run the authorization flow for you. That flow has
to redirect to a URI registered with *your* application on *your* domain, so the
refresh token is something you generate and paste in.

## Adding the connection

Open **Settings -> Connectors -> Connections** and select **Add Tesla
connection**, then provide:

| Field | Meaning |
|-------|---------|
| Tesla account region | Must match the region of the Tesla account. A token issued for one region is rejected by the others. |
| Client ID | From your registered developer application. |
| Refresh token | From your authorization-code exchange. |

Use **Test connection** to confirm the credentials work. It lists the vehicles
on the account with their current connectivity state.

Region determines the API host Moltis calls:

| Region | Fleet API host |
|--------|----------------|
| North America / Asia-Pacific | `fleet-api.prd.na.vn.cloud.tesla.com` |
| Europe, Middle East & Africa | `fleet-api.prd.eu.vn.cloud.tesla.com` |
| China | `fleet-api.prd.cn.vn.cloud.tesla.cn` |

## Datasets: current state or history

A Tesla dataset stores one of two shapes, chosen when you create it.

**Current state** keeps one row per vehicle and replaces it on every sync. Use
it for "what is the charge level right now", "is it plugged in", "is it locked".

**History** appends one row per reading and retains the most recent
`maxSamples` per vehicle, up to 20000. Older readings are retired as new ones
arrive. Use it for "how has range changed over the year", "when did it charge
last month", "how many kilometres this quarter".

Both shapes let you pick which vehicles (by VIN, or all of them) and which data
groups to fetch:

- Charge and battery (`charge_state`)
- Climate and temperature (`climate_state`)
- Drive state (`drive_state`)
- Vehicle state — odometer, locks, software version, tyre pressures
  (`vehicle_state`)
- Vehicle configuration (`vehicle_config`)
- Display units (`gui_settings`)
- Precise location (`location_data`) — needs the `vehicle_location` scope

Fields Moltis does not model explicitly are still stored verbatim, so a
change on Tesla's side does not silently drop data.

## Sleeping vehicles and rate limits

Requesting vehicle data from a Tesla that is asleep wakes it, and a schedule
that does this repeatedly will drain the battery. Moltis therefore checks each
vehicle's connectivity state first and **skips** any car that is not online:

- In a state dataset, the previously stored reading is kept unchanged. The
  reading's `observedAt` stays at the time it was actually taken.
- In a history dataset, no sample is recorded for that sync.
- On a vehicle's very first sync while it is unreachable, a row is still written
  so the vehicle is visible, with its connectivity state and no data.

Tesla also rate-limits vehicle data requests. Prefer a schedule of 60 minutes or
more. Tesla's own recommendation for continuous data is Fleet Telemetry
streaming rather than polling; Moltis does not implement that yet.

## Reading the data

Trusted agents get a `tesla_connector` tool with four read-only operations:

| Operation | Returns |
|-----------|---------|
| `list_datasets` | Synchronized Tesla datasets |
| `list_vehicles` | One entry per vehicle with its latest reading |
| `get_vehicle` | The latest full reading for one VIN |
| `search_readings` | Retained readings, newest first, filterable by VIN and text |

The tool cannot sync, send commands, wake a vehicle, or read account
credentials. Like every connector, its results are labelled untrusted external
content.

## Storage and security

- Connections added in the web UI are stored in `<data_dir>/connectors.db`, not
  written back to `moltis.toml`.
- The refresh token is encrypted by the [vault](vault.md) when vault encryption
  is enabled and the vault is unlocked. It is never returned by the API; account
  views only report whether a credential is stored.
- Editing a connection without retyping the refresh token keeps the stored one.
- Vehicle location, odometer, and charge history are sensitive. Connector
  datasets are operator-wide: every trusted operator and agent allowed to read
  the dataset can read your car's location history. Enable `location_data` only
  if that is acceptable, and remove the dataset when its retained data is no
  longer needed.
- Removing the connection stops future syncs but does not delete already
  synchronized data. Remove the dataset for that.
