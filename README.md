# pineapplehub

## Debug

```bash
poetry run python main.py
```

### Known issue

Currently, you may meet these issues during debugging:

#### Occupied port

The console says:

```
ERROR:    [Errno 98] Address already in use
```

It's caused by NiceGUI's port monitoring machanism. To overcome it, simply you can (if it's safe):

```bash
pkill python
```

#### Failed to get original screen size

The console says:

```
NameError: name 'screen_w' is not defined
```

One should refresh the page first then upload the file.