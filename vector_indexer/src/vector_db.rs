use anyhow::Result;
use qdrant_client::{
    Qdrant, QdrantError,
    qdrant::{
        CreateCollectionBuilder, Distance, PointsOperationResponse, ScalarQuantizationBuilder,
        SearchParamsBuilder, SearchPointsBuilder, UpsertPoints, VectorParamsBuilder,
    },
};

use tracing::{error, info};

use vector_common::{Embedder, candle_bert::CandleBert};

#[cfg_attr(test, mockall::automock)]
#[allow(async_fn_in_trait)]
pub trait VectorDb {
    async fn collection_exists(&self, collection: &str) -> Result<bool>;

    async fn create_collection(&self, collection: &str) -> Result<()>;

    async fn search(&self, collection: &str, vector: Vec<f32>) -> Result<()>;

    async fn upsert_points<T: Into<UpsertPoints> + 'static>(
        &self,
        request: T,
    ) -> Result<PointsOperationResponse, QdrantError>;
}

impl VectorDb for Qdrant {
    async fn collection_exists(&self, collection: &str) -> Result<bool> {
        Ok(self.collection_exists(collection).await?)
    }

    async fn create_collection(&self, collection: &str) -> Result<()> {
        self.create_collection(
            CreateCollectionBuilder::new(collection)
                .vectors_config(VectorParamsBuilder::new(384, Distance::Cosine))
                .quantization_config(ScalarQuantizationBuilder::default()),
        )
        .await?;
        Ok(())
    }

    async fn search(&self, collection: &str, vector: Vec<f32>) -> Result<()> {
        self.search_points(
            SearchPointsBuilder::new(collection, vector, 10)
                .with_payload(true)
                .params(SearchParamsBuilder::default().hnsw_ef(128).exact(false)),
        )
        .await?;

        Ok(())
    }

    async fn upsert_points<T: Into<UpsertPoints>>(
        &self,
        request: T,
    ) -> Result<PointsOperationResponse, QdrantError> {
        Qdrant::upsert_points(self, request).await
    }
}

// impl Embedder for SentenceEmbeddingsModel {
//     fn embed<S:AsRef<str> +Send+Sync+'static,T:AsRef<[S]> +'static>(&mut self,input:T,_batch_size:Option<usize>,) -> anyhow::Result<Vec<Vec<f32>>> {
//         SentenceEmbeddingsModel::encode(self, input.as_ref()).context("encoding")
//     }
// }
// impl Embedder for TextEmbedding {
//     fn embed<S: AsRef<str> + Send + Sync + 'static, T: AsRef<[S]> + 'static>(
//         &mut self,
//         input: T,
//         batch_size: Option<usize>,
//     ) -> anyhow::Result<Vec<Vec<f32>>> {
//         TextEmbedding::embed(self, input, batch_size)
//     }
// }

pub fn load_embedding_model() -> impl Embedder {
    // ---------------------------
    // Initialize embedding model
    // ---------------------------

    CandleBert::new()
}
pub async fn setup_vector_db<T: VectorDb + Send + Sync>(
    client: &T,
    collection: &str,
) -> anyhow::Result<()> {
    if !client.collection_exists(collection).await? {
        client.create_collection(collection).await?;
    }
    Ok(())
}

pub async fn perform_query<T: VectorDb + Send + Sync, E: Embedder>(
    embedder: &mut E,
    client: &T,
    collection: &str,
    query: &'static str,
) -> anyhow::Result<()> {
    let query_embedding = embedder.embed(vec![query], None)?;

    match client.search(collection, query_embedding[0].clone()).await {
        Err(e) => error!(error = ?e, "search failed"),
        Ok(_) => info!("search succeeded"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use vector_common::MockEmbedder;

    use super::*;

    #[tokio::test]
    async fn test_setup_creates_collection_when_missing() {
        let mut mock = MockVectorDb::new();

        mock.expect_collection_exists()
            .with(mockall::predicate::eq("test"))
            .times(1)
            .returning(|_| Ok(false));

        mock.expect_create_collection()
            .with(mockall::predicate::eq("test"))
            .times(1)
            .returning(|_| Ok(()));

        setup_vector_db(&mock, "test").await.unwrap();
    }

    #[tokio::test]
    async fn test_setup_skips_creation_if_exists() {
        let mut mock = MockVectorDb::new();

        mock.expect_collection_exists().returning(|_| Ok(true));

        mock.expect_create_collection().times(0);

        setup_vector_db(&mock, "test").await.unwrap();
    }

    #[tokio::test]
    async fn test_perform_query_calls_search() {
        let mut db = MockVectorDb::new();
        let mut embedder = MockEmbedder::new();

        embedder
            .expect_embed::<&str, Vec<&str>>()
            .returning(|_, _| Ok(vec![vec![0.1, 0.2, 0.3]]));

        db.expect_search().times(1).returning(|_, _| Ok(()));

        perform_query(&mut embedder, &db, "test", "hello")
            .await
            .unwrap();
    }
}
