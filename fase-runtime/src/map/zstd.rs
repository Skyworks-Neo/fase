use compio::{fs::create_dir_all, runtime::spawn_blocking};

use super::*;

pub(crate) struct Zstd;

impl Transform for Zstd {
    async fn run(self, context: Context) -> Result<Vec<Artifact>> {
        create_dir_all(context.output_dir()).await?;

        let mut outputs = Vec::with_capacity(context.inputs().len());
        for input in context.inputs() {
            outputs
                .push(compress(input.as_path(), context.output_dir(), context.zstd_level()).await?);
        }

        Ok(outputs)
    }
}

async fn compress(input: &Path, output_dir: &Path, level: i32) -> Result<Artifact> {
    let output = zstd_path(input, output_dir)?;

    let input = input.to_owned();
    let destination = output.clone();
    spawn_blocking(move || {
        let input = std::fs::File::open(input)?;
        let output = std::fs::File::create(destination)?;
        // external crate
        ::zstd::stream::copy_encode(input, output, level)
    })
    .await
    .map_err(|error| std::io::Error::other(format!("zstd task failed: {error:?}")))??;

    Ok(Artifact::from(output))
}

fn zstd_path(input: &Path, output_dir: &Path) -> std::io::Result<PathBuf> {
    let name = input.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("input path has no file name: {}", input.display()),
        )
    })?;

    Ok(output_dir.join(name).with_extension(zstd_extension(input)))
}

fn zstd_extension(input: &Path) -> String {
    match input.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => format!("{extension}.zst"),
        None => "zst".to_owned(),
    }
}
