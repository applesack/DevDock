# DevDock

DevDock is a Windows-only Tauri tray application for supervising local development services.

## Development

```powershell
npm install
npm run tauri dev
```

The first launch creates:

```text
%APPDATA%\DevDock\devdock.config.json
```

Logs default to:

```text
%APPDATA%\DevDock\logs
```

Left-click the tray icon to show the small status window. Right-click it to manage services.

## MVP Notes

- `process` and `react-native` services support start, stop, restart, logging, status patterns, and stdin actions.
- `windows-service` uses `sc.exe` for query, start, and stop.
- Log rotation fields are accepted but rotation is not implemented yet.
