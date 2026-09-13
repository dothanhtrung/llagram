<div align="center">

LLAGRAM
=======

[![pipeline status](https://gitlab.com/kimtinh/llagram/badges/master/pipeline.svg)](https://gitlab.com/kimtinh/llagram/-/commits/master)

[![Gitlab](https://img.shields.io/badge/gitlab-%23181717.svg?style=for-the-badge&logo=gitlab&logoColor=white)](https://gitlab.com/kimtinh/llagram)
[![Github](https://img.shields.io/badge/github-%23121011.svg?style=for-the-badge&logo=github&logoColor=white)](https://github.com/dothanhtrung/llagram)

![](./screenshots/1.png)

</div>


A telegram bot that helps you chat with your local llama.cpp.

Features:
* [x] Chat with local llama.cpp through Telegram.
* [x] Limit who can chat with the Telegram bot.
* [x] Memory of each chat thread is stored separately in SQlite.
* [x] Web fetch/search support.
* [x] REST API for other applications to send message to Telegram or local LLM.

How to use
----------

Download prebuilt binary from release page or build it by your self:
```shell
cargo build --release
```

Modify the `llagram.ron` to match with your local environment.

Run the bot:
```shell
./llagram -c ./llagram.ron
```


------

<div align="center">

![llagram](https://count.getloli.com/@git_llagram?name=git_llagram&theme=random&padding=9&offset=0&align=top&scale=1&pixelated=1&darkmode=auto)

</div>
