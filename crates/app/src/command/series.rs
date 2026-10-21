use oeuvre_core::{NewSeries, Series, StoreError};

use crate::event::{self, Event};
use crate::{AppState, CommandError, Handler, Outcome};

#[derive(Debug, Clone)]
pub struct CreateSeries {
    pub new: NewSeries,
}

pub(crate) struct Created(pub Series);

impl From<Created> for Outcome {
    fn from(Created(series): Created) -> Self {
        Outcome::CreateSeries(series)
    }
}

fn map_create_err(err: StoreError) -> CommandError {
    if err.is_invalid() {
        CommandError::Invalid(err.message.clone())
    } else {
        CommandError::Store(err)
    }
}

#[async_trait::async_trait]
impl Handler for CreateSeries {
    const NAME: &'static str = "create_series";
    type Output = Created;

    fn annotate(&self, ev: &mut Event) {
        ev.set(event::SERIES_TITLE, self.new.title.as_str().to_owned());
    }

    async fn run(self, state: &AppState, ev: &mut Event) -> Result<Created, CommandError> {
        let series = state
            .store()
            .create_series(self.new)
            .await
            .map_err(map_create_err)?;
        ev.set(event::SERIES_ID, series.id.to_string());
        ev.set(event::SERIES_TITLE, series.title.as_str().to_owned());
        Ok(Created(series))
    }
}
