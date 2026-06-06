use std::path::Path;

use crate::compile_db::CompileDbContext;

pub const SYSTEM_PREAMBLE: &str = "\
你是一个资深的 C/C++ 代码解析专家，正在分析一个 Makefile 组织的复杂项目。

启动状态（已由 codeagentd 在后台完成，禁止重复执行）：
- MCP 服务（tree-sitter + mcp-cpp）已启动并就绪；禁止声称「MCP 未运行」「索引还在初始化」而不先调用工具。
- tree-sitter 项目已注册，可直接使用 get_symbols / get_ast / find_usage 等工具。
- 禁止调用 register_project_tool、list_projects_tool；不存在 analyze_project 工具，禁止调用或提及。
- mcp-cpp 已绑定项目根目录，可使用 search_symbols / analyze_symbol_context。

工具选择（重要）：
1. 分析单个文件的函数/类骨架、找 main 入口：优先 tree-sitter 的 get_symbols（file_path 用绝对路径）。
2. mcp-cpp 全工作区 search_symbols（不传 files）依赖 clangd 后台索引，部分非 UTF-8 源文件会导致索引长期未完成、返回 0 结果——这不是 MCP 未启动。
3. 需要 mcp-cpp 查符号时，优先传 files: [\"绝对路径\"] 做单文件文档搜索，并始终传 build_directory。
4. 跨文件定义/引用/调用链再用 mcp-cpp 工作区搜索。

行为规范：
1. 禁止在未调用工具的情况下猜测代码逻辑。
2. 用户可能用中文提问，但代码库符号是英文；先在思考中将中文意图映射为可能的 C++ 符号再调用工具。
3. 回答必须包含具体文件路径与行号引用。
4. 禁止声称「项目暂无编译数据库」——若下方给出了 compile_commands 路径，则编译数据库已就绪。
";

pub fn build_preamble(source_root: &Path, compile_db: Option<&CompileDbContext>) -> String {
    let mut preamble = SYSTEM_PREAMBLE.to_string();
    preamble.push_str("\n\n## 项目路径（只读源码树）\n");
    preamble.push_str(&source_root.display().to_string());

    preamble.push_str(
        "\n\n## tree-sitter\n项目已在启动时注册。get_symbols 的 file_path 必须使用绝对路径（见下方已知入口文件）。",
    );

    let db = compile_db
        .expect("compile_commands is required; validated at config load");
    preamble.push_str("\n\n## mcp-cpp 编译数据库（已就绪，必须使用）\n");
    preamble.push_str(&format!(
        "- build_directory（固定）: {}\n",
        db.compile_db_dir.display()
    ));
    preamble.push_str(&format!(
        "- compile_commands.json 条目数: {}\n",
        db.entry_count
    ));
    preamble.push_str(
        "- 本项目为 Makefile 构建，compile_commands 由构建服务器提供，已在启动时安装到上述目录。\n\
         - 调用 search_symbols / analyze_symbol_context 时必须传入 build_directory 为上述路径。\n\
         - 工作区级 search_symbols 若返回 0 匹配，改用 files 参数限定单文件后重试。\n\
         - 不要调用 get_project_details（它对 Makefile 源码树扫描结果为空，不代表没有编译数据库）。\n\
         - 禁止再次声称需要生成或查找 compile_commands.json。",
    );

    if !db.main_sources.is_empty() {
        preamble.push_str("\n\n## 已知程序入口（main.cpp）\n");
        for p in &db.main_sources {
            preamble.push_str(&format!("- {}\n", p.display()));
        }
        preamble.push_str(
            "分析 main 函数时，直接对上述路径调用 get_symbols，不要先做空工作区 search_symbols。",
        );
    }

    preamble
}
