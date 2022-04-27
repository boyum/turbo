mod nft_json;

use anyhow::{anyhow, Context, Result};
use async_std::task::{block_on, spawn};
use clap::Parser;
use std::{
    collections::BTreeSet, env::current_dir, fs, future::Future, path::PathBuf, pin::Pin,
    sync::Arc, time::Instant,
};
use turbo_tasks::{stats::Stats, viz, NothingVc, TaskId, TurboTasks};
use turbo_tasks_fs::{
    glob::GlobVc, DirectoryEntry, DiskFileSystemVc, FileSystemPathVc, FileSystemVc,
    ReadGlobResultVc,
};
use turbopack::{
    all_assets, asset::AssetVc, emit, module, rebase::RebasedAssetVc, source_asset::SourceAssetVc,
};

use crate::nft_json::NftJsonAssetVc;

#[derive(clap::Args, Debug, Clone)]
struct CommonArgs {
    input: Vec<String>,

    #[clap(short, long)]
    visualize_graph: bool,

    #[clap(short, long)]
    watch: bool,
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
enum Args {
    // Print all files that the input files reference
    Print {
        #[clap(flatten)]
        common: CommonArgs,

        #[clap(short, long)]
        context_directory: Option<String>,
    },

    // Adds a *.nft.json file next to each input file which lists the referenced files
    Annotate {
        #[clap(flatten)]
        common: CommonArgs,

        #[clap(short, long)]
        context_directory: Option<String>,
    },

    // Copy input files and all referenced files to the output directory
    Build {
        #[clap(flatten)]
        common: CommonArgs,

        #[clap(short, long)]
        context_directory: Option<String>,

        #[clap(short, long, default_value_t = String::from("dist"))]
        output_directory: String,
    },

    // Print total size of input and referenced files
    Size {
        #[clap(flatten)]
        common: CommonArgs,

        #[clap(short, long)]
        context_directory: Option<String>,
    },
}

impl Args {
    fn common(&self) -> CommonArgs {
        match self {
            Args::Print { common, .. }
            | Args::Annotate { common, .. }
            | Args::Build { common, .. }
            | Args::Size { common, .. } => common.clone(),
        }
    }
}

async fn create_fs(name: &str, context: &str, watch: bool) -> Result<FileSystemVc> {
    let fs = DiskFileSystemVc::new(name.to_string(), context.to_string());
    if watch {
        fs.await?.start_watching()?;
    }
    Ok(fs.into())
}

async fn add_glob_results(result: ReadGlobResultVc, list: &mut Vec<AssetVc>) -> Result<()> {
    let result = result.await?;
    for entry in result.results.values() {
        match entry {
            DirectoryEntry::File(path) => {
                let source = SourceAssetVc::new(*path).into();
                list.push(module(source));
            }
            _ => {}
        }
    }
    for result in result.inner.values() {
        fn recurse<'a>(
            result: ReadGlobResultVc,
            list: &'a mut Vec<AssetVc>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
            Box::pin(add_glob_results(result, list))
        }
        // Boxing for async recursion
        recurse(*result, list).await?;
    }
    Ok(())
}

async fn input_to_modules<'a>(fs: FileSystemVc, input: &'a Vec<String>) -> Result<Vec<AssetVc>> {
    let root = FileSystemPathVc::new(fs, "");
    let mut list = Vec::new();
    for input in input.iter() {
        let glob = GlobVc::new(input);
        add_glob_results(root.read_glob(glob, false), &mut list).await?;
    }
    Ok(list)
}

fn process_context(dir: &PathBuf, context_directory: Option<String>) -> Result<String> {
    let mut context = PathBuf::from(context_directory.unwrap_or_else(|| ".".to_string()));
    if !context.is_absolute() {
        context = dir.join(context);
    }
    // context = context.canonicalize().unwrap();
    Ok(context
        .to_str()
        .ok_or_else(|| anyhow!("context directory contains invalid characters"))
        .unwrap()
        .to_string())
}

fn make_relative_path(dir: &PathBuf, context: &str, input: &str) -> Result<String> {
    let mut input = PathBuf::from(input);
    if !input.is_absolute() {
        input = dir.join(input);
    }
    // input = input.canonicalize()?;
    let input = input.strip_prefix(&context).with_context(|| {
        anyhow!(
            "{} is not part of the context directory {}",
            input.display(),
            context
        )
    })?;
    Ok(input
        .to_str()
        .ok_or_else(|| anyhow!("input contains invalid characters"))?
        .replace("\\", "/"))
}

fn process_input(dir: &PathBuf, context: &String, input: Vec<String>) -> Result<Vec<String>> {
    input
        .into_iter()
        .map(|input| make_relative_path(dir, context, &input))
        .collect()
}

fn main() {
    register();

    let args = Args::parse();
    let CommonArgs {
        input,
        visualize_graph,
        watch,
    } = args.common();

    let start = Instant::now();
    let finish = |tt: Arc<TurboTasks>, root_task: TaskId| {
        if watch {
            let handle = spawn({
                let tt = tt.clone();
                async move {
                    tt.wait_done().await;
                    println!("done in {} ms", start.elapsed().as_millis());

                    loop {
                        let (elapsed, count) = tt.wait_done().await;
                        if elapsed.as_millis() >= 10 {
                            println!("updated {} tasks in {} ms", count, elapsed.as_millis());
                        } else {
                            println!("updated {} tasks in {} µs", count, elapsed.as_micros());
                        }
                    }
                }
            });

            block_on(handle);
        } else {
            block_on(tt.wait_done());
            println!("done in {} ms", start.elapsed().as_millis());
            if visualize_graph {
                let mut stats = Stats::new();
                let guard = tt.guard();
                for task in tt.cached_tasks_iter(&guard) {
                    stats.add(&tt, &task);
                }
                stats.add_id(&tt, root_task);
                stats.merge_resolve();
                let tree = stats.treeify();
                let graph = viz::visualize_stats_tree(tree);
                fs::write("graph.html", viz::wrap_html(&graph)).unwrap();
                println!("graph.html written");
            }
        }
    };

    match args {
        Args::Print {
            context_directory,
            common: _,
        } => {
            let dir = current_dir().unwrap();
            let context = process_context(&dir, context_directory).unwrap();
            let input = process_input(&dir, &context, input).unwrap();
            let tt = TurboTasks::new();
            let task = tt.spawn_root_task(move || {
                let context = context.clone();
                let input = input.clone();
                Box::pin(async move {
                    let mut result = BTreeSet::new();
                    let fs = create_fs("context directory", &context, watch).await?;
                    let modules = input_to_modules(fs, &input).await?;
                    for module in modules {
                        let set = all_assets(module);
                        for asset in set.await?.assets.iter() {
                            let path = asset.path().await?;
                            result.insert(path.path.to_string());
                        }
                    }
                    for path in result {
                        println!("{}", path);
                    }
                    Ok(NothingVc::new().into())
                })
            });
            finish(tt, task);
        }
        Args::Annotate {
            context_directory,
            common: _,
        } => {
            let dir = current_dir().unwrap();
            let context = process_context(&dir, context_directory).unwrap();
            let input = process_input(&dir, &context, input).unwrap();
            let tt = TurboTasks::new();
            let task = tt.spawn_root_task(move || {
                let context = context.clone();
                let input = input.clone();
                Box::pin(async move {
                    let fs = create_fs("context directory", &context, watch).await?;
                    for module in input_to_modules(fs, &input).await? {
                        let nft_asset = NftJsonAssetVc::new(module).into();
                        emit(nft_asset)
                    }
                    Ok(NothingVc::new().into())
                })
            });
            finish(tt, task);
        }
        Args::Build {
            context_directory,
            output_directory,
            common: _,
        } => {
            let dir = current_dir().unwrap();
            let context = process_context(&dir, context_directory).unwrap();
            let output = process_context(&dir, Some(output_directory)).unwrap();
            let input = process_input(&dir, &context, input).unwrap();
            let tt = TurboTasks::new();
            let task = tt.spawn_root_task(move || {
                let context = context.clone();
                let input = input.clone();
                let output = output.clone();
                Box::pin(async move {
                    let fs = create_fs("context directory", &context, watch).await?;
                    let out_fs = create_fs("output directory", &output, watch).await?;
                    let input_dir = FileSystemPathVc::new(fs, "");
                    let output_dir = FileSystemPathVc::new(out_fs, "");
                    for module in input_to_modules(fs, &input).await? {
                        let rebased = RebasedAssetVc::new(module, input_dir, output_dir).into();
                        emit(rebased);
                    }
                    Ok(NothingVc::new().into())
                })
            });
            finish(tt, task);
        }
        Args::Size {
            context_directory,
            common: _,
        } => todo!(),
    }
}

fn register() {
    turbo_tasks::register();
    turbo_tasks_fs::register();
    turbopack::register();
    include!(concat!(env!("OUT_DIR"), "/register.rs"));
}
