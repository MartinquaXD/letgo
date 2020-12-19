use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct PortfolioItem {
    pub target_price: f64,
    pub set_number: String,
}

impl PartialEq for PortfolioItem {
    fn eq(&self, other: &Self) -> bool {
        self.set_number == other.set_number
    }
}

impl Eq for PortfolioItem {}

impl Hash for PortfolioItem {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.set_number.hash(hasher);
    }
}

#[derive(Debug, Clone)]
pub struct EbayResult {
    pub price: f64,
    pub date: chrono::DateTime<chrono::FixedOffset>,
    pub name: String,
}

pub struct PriceAnalysis {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub data_points: usize,
}

impl Default for PriceAnalysis {
    fn default() -> Self {
        PriceAnalysis {
            min: f64::MAX,
            max: 0.0,
            avg: 0.0,
            data_points: 0,
        }
    }
}



