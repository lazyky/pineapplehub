# pineapplehub

## Debug

```bash
poetry run python main.py
```

### Known issue

Currently, you may meet these issues during debugging:

#### `ERROR:    [Errno 98] Address already in use`

**Solution**

It's caused by NiceGUI's port monitoring machanism. To overcome it, simply you can (if it's safe):

```bash
pkill -9 python
```

#### `Unable to connect to VS Code server`

```
Unable to connect to VS Code server: Error in request.
Error: connect ENOENT /run/user/1000/vscode-ipc-b4272caf-f67a-4696-93b0-a7bcf845f5ce.sock
    at PipeConnectWrap.afterConnect [as oncomplete] (node:net:1607:16) {
  errno: -2,
  code: 'ENOENT',
  syscall: 'connect',
  address: '/run/user/1000/vscode-ipc-b4272caf-f67a-4696-93b0-a7bcf845f5ce.sock'
}
```

**Solution**

1. Close all VS code client.
2. SSH into the host
3. Run:
  ```bash
  ps -fu $USER | grep vscode | grep -v grep | awk '{print $2}' | xargs kill
  ```
