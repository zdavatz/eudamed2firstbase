/// Country code: EUDAMED ISO alpha-2 → GS1 numeric
pub fn country_alpha2_to_numeric(code: &str) -> &str {
    match code {
        "AT" => "040",
        "BE" => "56",
        "BG" => "100",
        "CY" => "196",
        "CZ" => "203",
        "DE" => "276",
        "DK" => "208",
        "EE" => "233",
        "EL" => "300",
        "ES" => "724",
        "FI" => "246",
        "FR" => "250",
        "HR" => "191",
        "HU" => "348",
        "IE" => "372",
        "IS" => "352",
        "IT" => "380",
        "LI" => "438",
        "LT" => "440",
        "LU" => "442",
        "LV" => "428",
        "MT" => "470",
        "NL" => "528",
        "NO" => "578",
        "PL" => "616",
        "PT" => "620",
        "RO" => "642",
        "SE" => "752",
        "SI" => "705",
        "SK" => "703",
        "CH" => "756",
        "TR" => "792",
        "XI" => "826", // Northern Ireland (UK)
        other => {
            eprintln!("Warning: unknown country code '{}', passing through", other);
            other
        }
    }
}

/// Risk class: EUDAMED → GS1 (additionalTradeItemClassificationSystemCode = 76)
pub fn risk_class_to_gs1(code: &str) -> &str {
    match code {
        "CLASS_I" => "EU_CLASS_I",
        "CLASS_IIA" => "EU_CLASS_IIA",
        "CLASS_IIB" => "EU_CLASS_IIB",
        "CLASS_III" => "EU_CLASS_III",
        "CLASS_A" => "EU_CLASS_A",
        "CLASS_B" => "EU_CLASS_B",
        "CLASS_C" => "EU_CLASS_C",
        "CLASS_D" => "EU_CLASS_D",
        other => other,
    }
}

/// Device status: EUDAMED → GS1
pub fn device_status_to_gs1(code: &str) -> &str {
    match code {
        "ON_THE_MARKET" | "ON_MARKET" => "ON_MARKET",
        "NO_LONGER_PLACED_ON_THE_MARKET" | "NO_LONGER_ON_THE_MARKET" => "NO_LONGER_PLACED_ON_MARKET",
        "NOT_INTENDED_FOR_EU_MARKET" => "NOT_INTENDED_FOR_EU_MARKET",
        other => other,
    }
}

/// Production identifier: EUDAMED → GS1
pub fn production_identifier_to_gs1(code: &str) -> &str {
    match code {
        "SERIALISATION_NUMBER" => "SERIAL_NUMBER",
        "BATCH_NUMBER" => "BATCH_NUMBER",
        "MANUFACTURING_DATE" => "MANUFACTURING_DATE",
        "EXPIRATION_DATE" => "EXPIRATION_DATE",
        "SOFTWARE_IDENTIFICATION" => "SOFTWARE_IDENTIFICATION",
        other => other,
    }
}

/// Substance type: EUDAMED → GS1 regulatedChemicalTypeCode
pub fn substance_type_to_gs1(code: &str) -> &str {
    match code {
        "MEDICINAL_PRODUCT_SUBSTANCE" => "MEDICINAL_PRODUCT",
        "HUMAN_PRODUCT_SUBSTANCE" => "HUMAN_PRODUCT",
        other => other,
    }
}

/// Clinical size type: EUDAMED CST code → GS1 clinicalSizeTypeCode
pub fn clinical_size_type_to_gs1(code: &str) -> &str {
    match code {
        "CST1" => "ACIDITY_PH",
        "CST2" => "FINGERS_AMOUNT",
        "CST3" => "ANGLE",
        "CST4" => "BEVEL",
        "CST5" => "CONCENTRATION",
        "CST6" => "CANNULA_WALL",
        "CST7" => "CAPACITY",
        "CST8" => "COATING",
        "CST9" => "DIAMETER",
        "CST10" => "DIAMETER_INNER",
        "CST11" => "OUTER_DIAMETER",
        "CST12" => "POLE_DISTANCE",
        "CST13" => "FLOW_RATE",
        "CST14" => "NEEDLE_GAUGE",
        "CST15" => "GUIDEWIRE_TYPE",
        "CST16" => "INFLATION_VOLUME",
        "CST17" => "BODY_SIDE",
        "CST18" => "BALLOON_LENGTH",
        "CST19" => "LENGTH",
        "CST20" => "LUMINOUS_FLUX",
        "CST21" => "MICROPARTICLE_SIZE",
        "CST22" => "NOMINAL_CAPACITY",
        "CST23" => "ELECTRODES_NUMBER",
        "CST24" => "PORE_SIZE",
        "CST25" => "PRESSURE",
        "CST26" => "SHAPE_FORM",
        "CST27" => "SIZE",
        "CST28" => "GUIDEWIRE_STIFFNESS",
        "CST29" => "STRENGTH",
        "CST30" => "AREA_SURFACE_AREA",
        "CST31" => "TIP_FIXATION_ANCHORING_ACTIVE",
        "CST32" => "TOTAL_VOLUME",
        "CST33" => "WIDTH",
        "CST34" => "WEIGHT",
        "CST35" => "TYPE_OF_PATIENT",
        "CST36" => "WAVELENGTH",
        "CST37" => "FREQUENCY",
        "CST38" => "OPTICAL_POWER",
        "CST39" => "CYLINDER_POWER",
        "CST40" => "ADDITION_POWER",
        "CST41" => "CYLINDER_AXIS",
        "CST42" => "BASE_CURVE",
        "CST43" => "OPTICAL_ZONE_DIAMETER",
        "CST44" => "POWER_PROFILE",
        "CST45" => "COLOUR",
        "CST46" => "EDGE_LIFT",
        "CST47" => "PRISM",
        "CST48" => "CEL",
        "CST49" => "RADIUS",
        "CST50" => "TANGENT",
        "CST51" => "HEIGHT",
        "CST52" => "CENTRE_THICKNESS",
        "CST53" => "TRUNCATION",
        "CST54" => "TRUNCATION_AXIS",
        "CST55" => "EDGE_RADIUS",
        "CST56" => "BODY_WEIGHT_KG",
        "CST57" => "BACK_CYLINDER_POWER",
        "CST58" => "BACK_CYLINDER_AXIS",
        "CST59" => "OPTICAL_ZONE_DIAMETER_BACK",
        "CST60" => "PRISM_AXIS",
        "CST61" => "TANGENT_STEEP",
        "CST62" => "HEIGHT_STEEP",
        "CST63" => "DIRECTION_OF_VIEW",
        "CST65" => "CIRCUMFERENCE",
        "CST66" => "DEPTH",
        "CST67" => "ENZYME_CATALYTIC_ACTIVITY",
        "CST999" => "DEVICE_SIZE_TEXT_SPECIFY",
        other => other,
    }
}

/// Measurement unit: EUDAMED MU code → GS1 UN/CEFACT code
pub fn measurement_unit_to_gs1(code: &str) -> &str {
    match code {
        "MU01" => "P1",
        "MU02" => "/L",
        "MU03" => "/mL",
        "MU04" => "/mmol",
        "MU05" => "NIU",
        "MU06" => "[iU]/d",
        "MU07" => "[iU]/L",
        "MU08" => "[iU]/mL",
        "MU09" => "CLT",
        "MU10" => "CMT",
        "MU11" => "2M",
        "MU12" => "CMQ",
        "MU13" => "MMQ",
        "MU14" => "G21",
        "MU15" => "DAY",
        "MU16" => "DLT",
        "MU17" => "DMT",
        "MU18" => "CEL",
        "MU19" => "umol/min",
        "MU20" => "A71",
        "MU21" => "Q32",
        "MU22" => "fmol/L",
        "MU23" => "FOT",
        "MU24" => "GRM",
        "MU25" => "GL",
        "MU26" => "HUR",
        "MU27" => "HTZ",
        "MU28" => "INH",
        "MU29" => "KGM",
        "MU30" => "K6",
        "MU31" => "KMH",
        "MU32" => "KPA",
        "MU33" => "kU/L",
        "MU34" => "LTR",
        "MU35" => "m[iU]/L",
        "MU36" => "MTR",
        "MU37" => "MGM",
        "MU38" => "mg/L",
        "MU39" => "mg/mL",
        "MU40" => "MC",
        "MU41" => "ug/min",
        "MU42" => "4G",
        "MU43" => "4H",
        "MU44" => "FH",
        "MU45" => "umol/L",
        "MU46" => "MBR",
        "MU47" => "MEQ",
        "MU48" => "MLT",
        "MU49" => "mL/s",
        "MU50" => "MMT",
        "MU51" => "mm[Hg]",
        "MU52" => "C18",
        "MU53" => "mmol/L",
        "MU54" => "C26",
        "MU55" => "MIN",
        "MU56" => "mL/d",
        "MU57" => "mL/min",
        "MU58" => "H67",
        "MU59" => "mmol/g",
        "MU60" => "mmol/kg",
        "MU61" => "mmol/kg[H2O]",
        "MU62" => "C34",
        "MU63" => "MON",
        "MU64" => "X_NGM",
        "MU65" => "Q34",
        "MU66" => "C45",
        "MU67" => "ng/L",
        "MU68" => "ng/mL",
        "MU69" => "nmol/d",
        "MU70" => "nmol/g",
        "MU71" => "nmol/h/mL",
        "MU72" => "nmol/L",
        "MU73" => "pg",
        "MU74" => "pg/mL",
        "MU75" => "Q33",
        "MU76" => "C52",
        "MU77" => "pmol/g",
        "MU78" => "pmol/h/mg",
        "MU79" => "pmol/h/mL",
        "MU80" => "pmol/L",
        "MU81" => "SEC",
        "MU82" => "CMK",
        "MU83" => "FTK",
        "MU84" => "INK",
        "MU85" => "MTK",
        "MU86" => "MMK",
        "MU88" => "U/h",
        "MU89" => "U/(12.h)",
        "MU90" => "U/(2.h)",
        "MU91" => "U/d",
        "MU92" => "U/g",
        "MU93" => "U/kg",
        "MU94" => "U/mL",
        "MU95" => "u[iU]/mL",
        "MU96" => "ug/d",
        "MU97" => "ug/L",
        "MU98" => "ug/mL",
        "MU99" => "um/s",
        "MU100" => "umol/g",
        "MU101" => "WEE",
        "MU102" => "ANN",
        "MU103" => "WTT",
        "MU104" => "diop",
        "MU105" => "DD",
        "MU106" => "LUM",
        "MU107" => "AMP",
        "MU108" => "KEL",
        "MU109" => "cd",
        "MU110" => "NEW",
        "MU111" => "PAL",
        "MU112" => "JOU",
        "MU113" => "C",
        "MU114" => "VLT",
        "MU115" => "OHM",
        "MU116" => "S",
        "MU117" => "F",
        "MU118" => "Wb",
        "MU119" => "T",
        "MU120" => "H",
        "MU121" => "LUX",
        "MU122" => "BQL",
        "MU123" => "Gy",
        "MU124" => "Sv",
        "MU125" => "kat",
        "MU126" => "BAR",
        "MU127" => "eV",
        "MU128" => "u",
        "MU129" => "har",
        "MU130" => "TNE",
        "MU132" => "Np",
        "MU133" => "B",
        "MU134" => "2N",
        "MU135" => "ug/dL",
        "MU136" => "mg/dL",
        "MU169" => "Q30",
        "MU170" => "H79",
        other => other,
    }
}

/// Storage handling code: EUDAMED SHCnnn → GS1 SHCnn (strip leading zeros)
pub fn storage_handling_to_gs1(code: &str) -> String {
    if code.starts_with("SHC") {
        if let Ok(num) = code[3..].parse::<u32>() {
            return format!("SHC{:02}", num);
        }
    }
    code.to_string()
}

/// Regulatory act from risk class
pub fn regulation_from_risk_class(risk_class: &str) -> &str {
    match risk_class {
        "CLASS_I" | "CLASS_IIA" | "CLASS_IIB" | "CLASS_III" => "MDR",
        "CLASS_A" | "CLASS_B" | "CLASS_C" | "CLASS_D" => "IVDR",
        _ => "MDR",
    }
}

/// Classification system code for risk class
pub fn classification_system_for_risk_class(_risk_class: &str) -> &str {
    "76"
}
