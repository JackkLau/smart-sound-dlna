use std::{env, fs, path::PathBuf};
use xiaomi_dlna_desktop::movie_library::Store;

fn main() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let usage =
        "用法：movie_library_db [--from-bundle 预置片库路径] 用户数据库路径 [豆瓣快照.json]";
    let first = args.next().ok_or(usage)?;
    let (template, path) = if first == "--from-bundle" {
        (
            Some(PathBuf::from(args.next().ok_or(usage)?)),
            PathBuf::from(args.next().ok_or(usage)?),
        )
    } else {
        (None, PathBuf::from(first))
    };
    let extra = args.next();
    if args.next().is_some() {
        return Err("请选择一个数据库及至多一个导入文件".into());
    }
    let store = match template {
        Some(template) => Store::open_packaged(&path, &template)?,
        None => Store::open(&path)?,
    };
    if let Some(input) = extra {
        let json = fs::read_to_string(input).map_err(|e| format!("无法读取片单：{e}"))?;
        println!("新增或更新的达标电影：{}", store.import(&json)?);
    }
    let movies = store.list("")?;
    let with_link = movies.iter().filter(|m| m.resource_count > 0).count();
    println!(
        "{}：{} 部达标电影，其中 {} 部已收录资源。",
        path.display(),
        movies.len(),
        with_link
    );
    Ok(())
}
