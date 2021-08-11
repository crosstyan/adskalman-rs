use approx::assert_relative_eq;
use na::allocator::Allocator;
use na::dimension::DimMin;
use na::dimension::{U2, U4};
use na::OMatrix;
use na::{DefaultAllocator, RealField};
use nalgebra as na;
use nalgebra::{Const, OVector};
use serde::{Deserialize, Serialize};

use adskalman::{
    CovarianceUpdateMethod, KalmanFilterNoControl, ObservationModel, TransitionModelLinearNoControl,
};

// This data was generated by running `online_tracking.rs` in `examples`.
const TRACKING_DATA: &str = include_str!("data/online_tracking.csv");
// This data was generated by running `offline_smoothing.rs` in `examples`.
const SMOOTHED_DATA: &str = include_str!("data/offline_smoothing.csv");

#[derive(Debug, Serialize, Deserialize)]
struct CsvRow {
    t: f64,
    true_x: f64,
    true_y: f64,
    true_xvel: f64,
    true_yvel: f64,
    obs_x: f64,
    obs_y: f64,
    est_x: f64,
    est_y: f64,
    est_xvel: f64,
    est_yvel: f64,
}

// motion model -----------------

struct ConstantVelocity2DModel<R>
where
    R: RealField,
    DefaultAllocator: Allocator<R, U4, U4>,
    DefaultAllocator: Allocator<R, U4>,
{
    transition_model: OMatrix<R, U4, U4>,
    transition_model_transpose: OMatrix<R, U4, U4>,
    transition_noise_covariance: OMatrix<R, U4, U4>,
}

impl<R> ConstantVelocity2DModel<R>
where
    R: RealField + Copy,
{
    fn new(dt: R, noise_scale: R) -> Self {
        let one = na::convert(1.0);
        let zero = na::convert(0.0);
        // Create transition model. 2D position and 2D velocity.
        #[rustfmt::skip]
        let transition_model = OMatrix::<R,U4,U4>::new(one, zero,  dt, zero,
                            zero, one, zero,  dt,
                            zero, zero, one, zero,
                            zero, zero, zero, one);

        // This form is after N. Shimkin's lecture notes in
        // Estimation and Identification in Dynamical Systems
        // http://webee.technion.ac.il/people/shimkin/Estimation09/ch8_target.pdf
        // See also eq. 43 on pg. 13 of
        // http://www.robots.ox.ac.uk/~ian/Teaching/Estimation/LectureNotes2.pdf

        let t33 = dt * dt * dt / na::convert(3.0);
        let t22 = dt * dt / na::convert(2.0);
        #[rustfmt::skip]
        let transition_noise_covariance = OMatrix::<R,U4,U4>::new(t33, zero, t22, zero,
                                        zero, t33, zero, t22,
                                        t22, zero, dt, zero,
                                        zero, t22, zero, dt)*noise_scale;
        Self {
            transition_model,
            transition_model_transpose: transition_model.transpose(),
            transition_noise_covariance,
        }
    }
}

impl<R> TransitionModelLinearNoControl<R, U4> for ConstantVelocity2DModel<R>
where
    R: RealField,
    DefaultAllocator: Allocator<R, U4, U4>,
    DefaultAllocator: Allocator<R, U2, U4>,
    DefaultAllocator: Allocator<R, U4, U2>,
    DefaultAllocator: Allocator<R, U2, U2>,
    DefaultAllocator: Allocator<R, U4>,
{
    fn F(&self) -> &OMatrix<R, U4, U4> {
        &self.transition_model
    }
    fn FT(&self) -> &OMatrix<R, U4, U4> {
        &self.transition_model_transpose
    }
    fn Q(&self) -> &OMatrix<R, U4, U4> {
        &self.transition_noise_covariance
    }
}

// observation model ------------

struct PositionObservationModel<R: RealField>
where
    DefaultAllocator: Allocator<R, U4, U4>,
    DefaultAllocator: Allocator<R, U2, U4>,
    DefaultAllocator: Allocator<R, U4, U2>,
    DefaultAllocator: Allocator<R, U2, U2>,
    DefaultAllocator: Allocator<R, U4>,
{
    observation_matrix: OMatrix<R, U2, U4>,
    observation_matrix_transpose: OMatrix<R, U4, U2>,
    observation_noise_covariance: OMatrix<R, U2, U2>,
}

impl<R: RealField + Copy> PositionObservationModel<R> {
    fn new(var: R) -> Self {
        let one = na::convert(1.0);
        let zero = na::convert(0.0);
        // Create observation model. We only observe the position.
        #[rustfmt::skip]
        let observation_matrix = OMatrix::<R,U2,U4>::new(one, zero, zero, zero,
                                    zero, one, zero, zero);
        #[rustfmt::skip]
        let observation_noise_covariance = OMatrix::<R,U2,U2>::new(var, zero,
                                                zero, var);
        Self {
            observation_matrix,
            observation_matrix_transpose: observation_matrix.transpose(),
            observation_noise_covariance,
        }
    }
}

impl<R: RealField> ObservationModel<R, U4, U2> for PositionObservationModel<R>
where
    DefaultAllocator: Allocator<R, U4, U4>,
    DefaultAllocator: Allocator<R, U2, U4>,
    DefaultAllocator: Allocator<R, U4, U2>,
    DefaultAllocator: Allocator<R, U2, U2>,
    DefaultAllocator: Allocator<R, U4>,
    DefaultAllocator: Allocator<R, U2>,
    DefaultAllocator: Allocator<(usize, usize), U2>,
    U2: DimMin<U2, Output = U2>,
{
    fn H(&self) -> &OMatrix<R, U2, U4> {
        &self.observation_matrix
    }
    fn HT(&self) -> &OMatrix<R, U4, U2> {
        &self.observation_matrix_transpose
    }
    fn R(&self) -> &OMatrix<R, U2, U2> {
        &self.observation_noise_covariance
    }
}

fn check_covariance_update_method(covariance_update_method: &CovarianceUpdateMethod) {
    let dt = 0.01;
    let true_initial_state = OVector::<f64, U4>::new(0.0, 0.0, 10.0, -5.0);
    #[rustfmt::skip]
    let initial_covariance = OMatrix::<f64,U4,U4>::new(0.1, 0.0, 0.0, 0.0,
        0.0, 0.1, 0.0, 0.0,
        0.0, 0.0, 0.1, 0.0,
        0.0, 0.0, 0.0, 0.1);

    let motion_model = ConstantVelocity2DModel::new(dt, 100.0);
    let observation_model = PositionObservationModel::new(0.01);
    let kf = KalmanFilterNoControl::new(&motion_model, &observation_model);

    let mut previous_estimate =
        adskalman::StateAndCovariance::new(true_initial_state, initial_covariance);

    let maxerr = 1e-8;

    let rdr = csv::Reader::from_reader(TRACKING_DATA.as_bytes());
    for row in rdr.into_deserialize().into_iter() {
        let row: CsvRow = row.unwrap();
        println!("{:?}", row);
        let this_observation = OVector::<f64, Const<2>>::new(row.obs_x, row.obs_y);
        let this_estimate = kf
            .step_with_options(
                &previous_estimate,
                &this_observation,
                *covariance_update_method,
            )
            .unwrap();

        let this_state = this_estimate.state();
        println!("  -> {:?}\n\n", this_state);

        assert_relative_eq!(this_state[0], row.est_x, max_relative = maxerr);
        assert_relative_eq!(this_state[1], row.est_y, max_relative = maxerr);
        assert_relative_eq!(this_state[2], row.est_xvel, max_relative = maxerr);
        assert_relative_eq!(this_state[3], row.est_yvel, max_relative = maxerr);

        previous_estimate = this_estimate;
    }
}

#[test]
fn test_covariance_update_methods() {
    let forms = [
        CovarianceUpdateMethod::JosephForm,
        CovarianceUpdateMethod::OptimalKalman,
        CovarianceUpdateMethod::OptimalKalmanForcedSymmetric,
    ];
    for form in forms.iter() {
        check_covariance_update_method(form);
    }
}

#[test]
fn test_offline_smoothing() {
    let dt = 0.01;
    let true_initial_state = OVector::<f64, U4>::new(0.0, 0.0, 10.0, -5.0);
    #[rustfmt::skip]
    let initial_covariance = OMatrix::<f64,U4,U4>::new(0.1, 0.0, 0.0, 0.0,
        0.0, 0.1, 0.0, 0.0,
        0.0, 0.0, 0.1, 0.0,
        0.0, 0.0, 0.0, 0.1);

    let motion_model = ConstantVelocity2DModel::new(dt, 100.0);
    let observation_model = PositionObservationModel::new(0.01);
    let kf = KalmanFilterNoControl::new(&motion_model, &observation_model);

    let mut observation = vec![];
    let mut expected = vec![];

    let rdr = csv::Reader::from_reader(SMOOTHED_DATA.as_bytes());
    for row in rdr.into_deserialize().into_iter() {
        let row: CsvRow = row.unwrap();

        println!("{:?}", row);
        let this_observation = OVector::<f64, Const<2>>::new(row.obs_x, row.obs_y);
        observation.push(this_observation);
        expected.push(OVector::<f64, Const<4>>::new(
            row.est_x,
            row.est_y,
            row.est_xvel,
            row.est_yvel,
        ));
    }

    let initial_estimate =
        adskalman::StateAndCovariance::new(true_initial_state, initial_covariance);
    let actual = kf.smooth(&initial_estimate, &observation).unwrap();

    let maxerr = 1e-8;
    for (actual_row, expected_row) in actual.iter().zip(expected.iter()) {
        let this_state = actual_row.state();
        for i in 0..4 {
            assert_relative_eq!(this_state[i], expected_row[i], max_relative = maxerr);
        }
    }
}

#[test]
fn test_offline_smoothing_with_missing_data() {
    let dt = 0.01;
    let true_initial_state = OVector::<f64, U4>::new(0.0, 0.0, 10.0, -5.0);
    #[rustfmt::skip]
    let initial_covariance = OMatrix::<f64,U4,U4>::new(0.1, 0.0, 0.0, 0.0,
        0.0, 0.1, 0.0, 0.0,
        0.0, 0.0, 0.1, 0.0,
        0.0, 0.0, 0.0, 0.1);

    let motion_model = ConstantVelocity2DModel::new(dt, 100.0);
    let observation_model = PositionObservationModel::new(0.01);
    let kf = KalmanFilterNoControl::new(&motion_model, &observation_model);

    let mut observation = vec![];
    let mut expected = vec![];

    let rdr = csv::Reader::from_reader(SMOOTHED_DATA.as_bytes());
    for row in rdr.into_deserialize().into_iter() {
        let row: CsvRow = row.unwrap();

        println!("{:?}", row);
        let this_observation = OVector::<f64, Const<2>>::new(row.obs_x, row.obs_y);
        observation.push(this_observation);
        expected.push(OVector::<f64, Const<4>>::new(
            row.est_x,
            row.est_y,
            row.est_xvel,
            row.est_yvel,
        ));
    }

    assert_eq!(observation.len(), 50);
    for i in 25..30 {
        observation[i] = OVector::<f64, Const<2>>::new(std::f64::NAN, std::f64::NAN);
    }

    let initial_estimate =
        adskalman::StateAndCovariance::new(true_initial_state, initial_covariance);
    let actual = kf.smooth(&initial_estimate, &observation).unwrap();

    // We cannot be so precise because some data is missing.
    let maxerr = 1e-1;

    for (actual_row, expected_row) in actual.iter().zip(expected.iter()) {
        let this_state = actual_row.state();
        for i in 0..4 {
            assert_relative_eq!(this_state[i], expected_row[i], max_relative = maxerr);
        }
    }
}
