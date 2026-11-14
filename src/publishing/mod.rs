//! Publication of completed cards into learner-facing study artifacts.

mod study_package;

pub use study_package::IMAGE_STYLE;
pub(crate) use study_package::{StudyPackagePublisher, SystemPublicationClock};
