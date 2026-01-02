# Vendoring notes (Winderoo)

- **Upstream repo**: `https://github.com/diogo464/embassy-ha`
- **Upstream revision**: `5d3971cb2e7ffd5b9d6114bdec004ce30857fb3b` (branch `main` as of 2026-01-02)

## Local patches

- Increase `DeviceResources` entity limit (needed for full ArduinoHA parity in Winderoo).
- Add `select` entity support (`create_select`, `SelectConfig`, `Select`).
- Add text sensor support (`create_text_sensor`, `TextSensorConfig`, `TextSensor`).

