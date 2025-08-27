# Haruki Sekai API

**Haruki Sekai API** is a companion project for [HarukiBot](https://github.com/Team-Haruki), providing direct API access to various servers of the game `Project Sekai: Colorful Stage`.

## Requirements
+ `MySQL` or `SQLite`
+ `Redis`

## How to Use

1. Rename `configs.example.py` to `configs.py` and then edit it.
2. Install [uv](https://github.com/astral-sh/uv) to manage and install project dependencies.
3. Run the following command to install dependencies:
   ```bash
   uv sync
   ```
4. (Optional) If you plan to use MySQL via asyncmy, install:
   ```bash
   uv add asyncmy
   ```
5. (Optional) If you plan to use SQLite via aiosqlite, install:
   ```bash
   uv add aiosqlite
   ```
6. (Optional) If you're on **Linux/macOS**, it's recommended to install [uvloop](https://github.com/MagicStack/uvloop) for better performance:
   ```bash
   uv add uvloop
   ```
7. If you need to change the listening address or other server settings, edit the `hypercorn.toml` file. If you have installed uvloop, uncomment the `worker_class` line in `hypercorn.toml` to enable it.
8. Finally, run the server using:
   ```bash
   uv run hypercorn app:app --config hypercorn.toml
   ```

## License

This project is licensed under the MIT License.