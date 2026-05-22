use compio::{
    BufResult,
    fs::{create_dir_all, write},
};
use cyper::Client;
use url::Url;

use super::*;

pub(crate) struct Http;

impl Transform for Http {
    async fn run(self, context: Context) -> Result<Vec<Artifact>> {
        create_dir_all(context.output_dir()).await?;
        let client = Client::new()?;
        let mut outputs = Vec::with_capacity(context.inputs().len());
        for input in context.inputs() {
            outputs.push(fetch(&client, input.path(), context.output_dir()).await?);
        }
        Ok(outputs)
    }
}

async fn fetch(client: &Client, input: &Path, output_dir: &Path) -> Result<Artifact> {
    let url = input_url(input)?;
    let response = client.get(url)?.send().await?;
    let status = response.status();
    let final_url = response.url().clone();
    if !status.is_success() {
        return Err(Box::new(std::io::Error::other(format!(
            "get {final_url} failed with {status}"
        ))));
    }
    let output = output_dir.join(file_name(&final_url));
    let bytes = response.bytes().await?;
    let BufResult(result, _) = write(&output, bytes.to_vec()).await;
    result?;
    Ok(Artifact::from(output))
}

fn input_url(input: &Path) -> Result<&str> {
    input.to_str().ok_or_else(|| {
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("input is not valid: {}", input.display()),
        )) as Error
    })
}

fn file_name(url: &Url) -> &str {
    url.path_segments()
        .and_then(|segments| segments.rev().find(|segment| !segment.is_empty()))
        .filter(|name| !name.is_empty())
        .unwrap_or("index")
}
