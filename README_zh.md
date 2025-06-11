## command_tool(WIP)

一个可以帮你生成 command_line 的 rust 宏。

请注意当前功能未完善，请注意

### how to use?

我们最终将生成 clap 代码。所以请您在使用时手动引用 clap。

在引用本仓库后，在 `src/main.rs` 中按

```rust
use command_tool::generate_commands;

pub mod command;

generate_commands!{
    src = "src/command",
    exec_func = "exec"
}

fn main() {
    exec();
}
```

其中 `src` 的参数 对应的是 您执行对应命令行所在位置。

`exec_func` 对应的是 **最终生成的（匹配执行）的函数** 的名称。

#### 匹配的路由

`src/command/mod.rs` 中

```rust
//! router: <command_name>
//! description: <description>

pub mod start;

```

前两行中的 `//! router: ` 和 `//! description: ` 后填您的命令行的描述。

我们支持多层命令行（例如 `ctl set abc [OPTION]`）

您只需要在 `src/command/set/mod.rs`

中按上述 `src/command/mod.rs` 中前两行注释引入

```rust
//! router: set
//! description: settings

pub mod abc;
```

#### 命令设置

这是一个一般而言的 `src/command/start.rs` 代码：

```rust
//! router: start
//! description: Start the server
//! --config -c <config>, config path (optional\)
//! --port -p <port>, server port (optional\, default: read by config or 1824)
//! --worker -w <worker>, worker number (optional\, default: 4)


pub fn run(log_level: String, config: Option<String>, port: Option<String>, worker: Option<String>) {
    // your code .....
}
```

您的 `start.rs` 代码中必须包含一个 `run` 函数，在 `run` 函数中每个变量应为 `Option<T>` 或 `T` (`T` 应为 Clap 可以转化的类型如浮点类型、整数类型 或 `String`)