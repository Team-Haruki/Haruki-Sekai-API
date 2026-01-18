> Caution  
> This project is rewritten in Rust, Go edition and Python edition are not maintained anymore.  
> If you want to use Python edition, please go to [old python branch](https://github.com/Team-Haruki/Haruki-Sekai-API/tree/old-python)  
> If you want to use Go edition, please go to [old go branch](https://github.com/Team-Haruki/Haruki-Sekai-API/tree/old-go)

# Haruki Sekai API

**Haruki Sekai API** is a companion project for [HarukiBot](https://github.com/Team-Haruki), providing direct API access to various servers of the game `Project Sekai: Colorful Stage`.

## Requirements
+ `MySQL`, `SQLite`, `PostgreSQL` (Optional, depending on your database choice)
+ `Redis` (Optional, for caching sekai users)

## How to Use
1. Go to release page to download `haruki-sekai-api`
2. Rename `haruki-sekai-configs.example.yaml` to `haruki-sekai-configs.yaml` and then edit it.
3. Make a new directory or use an exists directory
4. Put `haruki-sekai-api` and `haruki-sekai-configs.yaml` in the same directory
5. Edit `haruki-sekai-configs.yaml` and configure it
6. Open Terminal, and `cd` to the directory
7. Run `haruki-sekai-api`

## License

This project is licensed under the MIT License.