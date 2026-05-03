/// Country code: EUDAMED ISO 3166-1 alpha-2 -> GS1 numeric
/// Source: GS1 UDI Connector Profile Overview Apr 2026 V1.1 (UDID_CodeLists tab,
/// salesConditionTargetMarketCountry/countryCode + structuredAddress/countryCode).
/// 250 ISO entries + GB alias (EUDAMED uses ISO GB; GS1's sheet uses non-standard UK).
/// Special case: XI (Northern Ireland) -> "XI" literal, not numeric (GS1 convention).
pub fn country_alpha2_to_numeric(code: &str) -> &str {
    match code {
        "AD" => "020", // ANDORRA
        "AE" => "784", // UNITED ARAB EMIRATES
        "AF" => "004", // AFGHANISTAN
        "AG" => "028", // ANTIGUA AND BARBUDA
        "AI" => "660", // ANGUILLA
        "AL" => "008", // ALBANIA
        "AM" => "051", // ARMENIA
        "AO" => "024", // ANGOLA
        "AQ" => "010", // ANTARCTICA
        "AR" => "032", // ARGENTINA
        "AS" => "016", // AMERICAN SAMOA
        "AT" => "040", // AUSTRIA
        "AU" => "036", // AUSTRALIA
        "AW" => "533", // ARUBA
        "AX" => "248", // ÅLAND ISLANDS
        "AZ" => "031", // AZERBAIJAN
        "BA" => "070", // BOSNIA AND HERZEGOVINA
        "BB" => "052", // BARBADOS
        "BD" => "050", // BANGLADESH
        "BE" => "056", // BELGIUM
        "BF" => "854", // BURKINA FASO
        "BG" => "100", // BULGARIA
        "BH" => "048", // BAHRAIN
        "BI" => "108", // BURUNDI
        "BJ" => "204", // BENIN
        "BL" => "652", // SAINT BARTHÉLEMY
        "BM" => "060", // BERMUDA
        "BN" => "096", // BRUNEI DARUSSALAM
        "BO" => "068", // BOLIVIA (PLURINATIONAL STATE OF)
        "BQ" => "535", // BONAIRE, SINT EUSTATIUS AND SABA
        "BR" => "076", // BRAZIL
        "BS" => "044", // BAHAMAS
        "BT" => "064", // BHUTAN
        "BV" => "074", // BOUVET ISLAND
        "BW" => "072", // BOTSWANA
        "BY" => "112", // BELARUS
        "BZ" => "084", // BELIZE
        "CA" => "124", // CANADA
        "CC" => "166", // COCOS (KEELING) ISLANDS
        "CD" => "180", // CONGO, DEMOCRATIC REPUBLIC OF THE
        "CF" => "140", // CENTRAL AFRICAN REPUBLIC
        "CG" => "178", // CONGO
        "CH" => "756", // SWITZERLAND
        "CI" => "384", // CÔTE D'IVOIRE
        "CK" => "184", // COOK ISLANDS
        "CL" => "152", // CHILE
        "CM" => "120", // CAMEROON
        "CN" => "156", // CHINA
        "CO" => "170", // COLOMBIA
        "CR" => "188", // COSTA RICA
        "CU" => "192", // CUBA
        "CV" => "132", // CABO VERDE
        "CW" => "531", // CURAÇAO
        "CX" => "162", // CHRISTMAS ISLAND
        "CY" => "196", // CYPRUS
        "CZ" => "203", // CZECHIA
        "DE" => "276", // GERMANY
        "DJ" => "262", // DJIBOUTI
        "DK" => "208", // DENMARK
        "DM" => "212", // DOMINICA
        "DO" => "214", // DOMINICAN REPUBLIC
        "DZ" => "012", // ALGERIA
        "EC" => "218", // ECUADOR
        "EE" => "233", // ESTONIA
        "EG" => "818", // EGYPT
        "EH" => "732", // WESTERN SAHARA
        "EL" => "300", // GREECE
        "ER" => "232", // ERITREA
        "ES" => "724", // SPAIN
        "ET" => "231", // ETHIOPIA
        "FI" => "246", // FINLAND
        "FJ" => "242", // FIJI
        "FK" => "238", // FALKLAND ISLANDS (MALVINAS)
        "FM" => "583", // MICRONESIA (FEDERATED STATES OF)
        "FO" => "234", // FAROE ISLANDS
        "FR" => "250", // FRANCE
        "GA" => "266", // GABON
        "GB" => "826", // UNITED KINGDOM OF GREAT BRITAIN AND NORTHERN  IRELAND
        "GD" => "308", // GRENADA
        "GE" => "268", // GEORGIA
        "GF" => "254", // FRENCH GUIANA
        "GG" => "831", // GUERNSEY
        "GH" => "288", // GHANA
        "GI" => "292", // GIBRALTAR
        "GL" => "304", // GREENLAND
        "GM" => "270", // GAMBIA
        "GN" => "324", // GUINEA
        "GP" => "312", // GUADELOUPE
        "GQ" => "226", // EQUATORIAL GUINEA
        "GS" => "239", // SOUTH GEORGIA AND THE SOUTH SANDWICH ISLANDS
        "GT" => "320", // GUATEMALA
        "GU" => "316", // GUAM
        "GW" => "624", // GUINEA-BISSAU
        "GY" => "328", // GUYANA
        "HK" => "344", // HONG KONG
        "HM" => "334", // HEARD ISLAND AND MCDONALD ISLANDS
        "HN" => "340", // HONDURAS
        "HR" => "191", // CROATIA
        "HT" => "332", // HAITI
        "HU" => "348", // HUNGARY
        "ID" => "360", // INDONESIA
        "IE" => "372", // IRELAND
        "IL" => "376", // ISRAEL
        "IM" => "833", // ISLE OF MAN
        "IN" => "356", // INDIA
        "IO" => "086", // BRITISH INDIAN OCEAN TERRITORY
        "IQ" => "368", // IRAQ
        "IR" => "364", // IRAN (ISLAMIC REPUBLIC OF)
        "IS" => "352", // ICELAND
        "IT" => "380", // ITALY
        "JE" => "832", // JERSEY
        "JM" => "388", // JAMAICA
        "JO" => "400", // JORDAN
        "JP" => "392", // JAPAN
        "KE" => "404", // KENYA
        "KG" => "417", // KYRGYZSTAN
        "KH" => "116", // CAMBODIA
        "KI" => "296", // KIRIBATI
        "KM" => "174", // COMOROS
        "KN" => "659", // SAINT KITTS AND NEVIS
        "KP" => "408", // KOREA (DEMOCRATIC PEOPLE'S REPUBLIC OF)
        "KR" => "410", // KOREA, REPUBLIC OF
        "KW" => "414", // KUWAIT
        "KY" => "136", // CAYMAN ISLANDS
        "KZ" => "398", // KAZAKHSTAN
        "LA" => "418", // LAO PEOPLE'S DEMOCRATIC REPUBLIC
        "LB" => "422", // LEBANON
        "LC" => "662", // SAINT LUCIA
        "LI" => "438", // LIECHTENSTEIN
        "LK" => "144", // SRI LANKA
        "LR" => "430", // LIBERIA
        "LS" => "426", // LESOTHO
        "LT" => "440", // LITHUANIA
        "LU" => "442", // LUXEMBOURG
        "LV" => "428", // LATVIA
        "LY" => "434", // LIBYA
        "MA" => "504", // MOROCCO
        "MC" => "492", // MONACO
        "MD" => "498", // MOLDOVA, REPUBLIC OF
        "ME" => "499", // MONTENEGRO
        "MF" => "663", // SAINT MARTIN (FRENCH PART)
        "MG" => "450", // MADAGASCAR
        "MH" => "584", // MARSHALL ISLANDS
        "MK" => "807", // MACEDONIA, THE FORMER YUGOSLAV REPUBLIC OF
        "ML" => "466", // MALI
        "MM" => "104", // MYANMAR
        "MN" => "496", // MONGOLIA
        "MO" => "446", // MACAO
        "MP" => "580", // NORTHERN MARIANA ISLANDS
        "MQ" => "474", // MARTINIQUE
        "MR" => "478", // MAURITANIA
        "MS" => "500", // MONTSERRAT
        "MT" => "470", // MALTA
        "MU" => "480", // MAURITIUS
        "MV" => "462", // MALDIVES
        "MW" => "454", // MALAWI
        "MX" => "484", // MEXICO
        "MY" => "458", // MALAYSIA
        "MZ" => "508", // MOZAMBIQUE
        "NA" => "516", // NAMIBIA
        "NC" => "540", // NEW CALEDONIA
        "NE" => "562", // NIGER
        "NF" => "574", // NORFOLK ISLAND
        "NG" => "566", // NIGERIA
        "NI" => "558", // NICARAGUA
        "NL" => "528", // NETHERLANDS
        "NO" => "578", // NORWAY
        "NP" => "524", // NEPAL
        "NR" => "520", // NAURU
        "NU" => "570", // NIUE
        "NZ" => "554", // NEW ZEALAND
        "OM" => "512", // OMAN
        "PA" => "591", // PANAMA
        "PE" => "604", // PERU
        "PF" => "258", // FRENCH POLYNESIA
        "PG" => "598", // PAPUA NEW GUINEA
        "PH" => "608", // PHILIPPINES
        "PK" => "586", // PAKISTAN
        "PL" => "616", // POLAND
        "PM" => "666", // SAINT PIERRE AND MIQUELON
        "PN" => "612", // PITCAIRN
        "PR" => "630", // PUERTO RICO
        "PS" => "275", // PALESTINE, STATE OF
        "PT" => "620", // PORTUGAL
        "PW" => "585", // PALAU
        "PY" => "600", // PARAGUAY
        "QA" => "634", // QATAR
        "RE" => "638", // RÉUNION
        "RO" => "642", // ROMANIA
        "RS" => "688", // SERBIA
        "RU" => "643", // RUSSIAN FEDERATION
        "RW" => "646", // RWANDA
        "SA" => "682", // SAUDI ARABIA
        "SB" => "090", // SOLOMON ISLANDS
        "SC" => "690", // SEYCHELLES
        "SD" => "729", // SUDAN
        "SE" => "752", // SWEDEN
        "SG" => "702", // SINGAPORE
        "SH" => "654", // SAINT HELENA, ASCENSION AND TRISTAN DA CUNHA
        "SI" => "705", // SLOVENIA
        "SJ" => "744", // SVALBARD AND JAN MAYEN
        "SK" => "703", // SLOVAKIA
        "SL" => "694", // SIERRA LEONE
        "SM" => "674", // SAN MARINO
        "SN" => "686", // SENEGAL
        "SO" => "706", // SOMALIA
        "SR" => "740", // SURINAME
        "SS" => "728", // SOUTH SUDAN
        "ST" => "678", // SAO TOME AND PRINCIPE
        "SV" => "222", // EL SALVADOR
        "SX" => "534", // SINT MAARTEN (DUTCH PART)
        "SY" => "760", // SYRIAN ARAB REPUBLIC
        "SZ" => "748", // ESWATINI
        "TC" => "796", // TURKS AND CAICOS ISLANDS
        "TD" => "148", // CHAD
        "TF" => "260", // FRENCH SOUTHERN TERRITORIES
        "TG" => "768", // TOGO
        "TH" => "764", // THAILAND
        "TJ" => "762", // TAJIKISTAN
        "TK" => "772", // TOKELAU
        "TL" => "626", // TIMOR-LESTE
        "TM" => "795", // TURKMENISTAN
        "TN" => "788", // TUNISIA
        "TO" => "776", // TONGA
        "TR" => "792", // TURKEY
        "TT" => "780", // TRINIDAD AND TOBAGO
        "TV" => "798", // TUVALU
        "TW" => "158", // TAIWAN, PROVINCE OF CHINA
        "TZ" => "834", // TANZANIA, UNITED REPUBLIC OF
        "UA" => "804", // UKRAINE
        "UG" => "800", // UGANDA
        "UK" => "826", // UNITED KINGDOM OF GREAT BRITAIN AND NORTHERN  IRELAND
        "UM" => "581", // UNITED STATES MINOR OUTLYING ISLANDS
        "US" => "840", // UNITED STATES OF AMERICA
        "UY" => "858", // URUGUAY
        "UZ" => "860", // UZBEKISTAN
        "VA" => "336", // HOLY SEE
        "VC" => "670", // SAINT VINCENT AND THE GRENADINES
        "VE" => "862", // VENEZUELA (BOLIVARIAN REPUBLIC OF)
        "VG" => "092", // VIRGIN ISLANDS (BRITISH)
        "VI" => "850", // VIRGIN ISLANDS (U.S.)
        "VN" => "704", // VIET NAM
        "VU" => "548", // VANUATU
        "WF" => "876", // WALLIS AND FUTUNA
        "WS" => "882", // SAMOA
        "XI" => "XI",  // UNITED KINGDOM (NORTHERN IRELAND)
        "YE" => "887", // YEMEN
        "YT" => "175", // MAYOTTE
        "ZA" => "710", // SOUTH AFRICA
        "ZM" => "894", // ZAMBIA
        "ZW" => "716", // ZIMBABWE
        other => {
            eprintln!("Warning: unknown country code '{}', passing through", other);
            other
        }
    }
}

/// Whether a country alpha-2 code is valid for GDSN market sales conditions.
/// GB/XI are excluded post-Brexit (G541: invalid country code in GDSN).
pub fn is_valid_gdsn_market_country(iso2: &str) -> bool {
    !matches!(iso2, "GB" | "XI")
}

/// Whether a country alpha-2 code is an EU or EEA member state.
/// Used for 097.020 fallback: ORIGINAL_PLACED should be an EU/EEA country.
pub fn is_eu_eea_country(iso2: &str) -> bool {
    matches!(
        iso2,
        "AT" | "BE"
            | "BG"
            | "CY"
            | "CZ"
            | "DE"
            | "DK"
            | "EE"
            | "ES"
            | "FI"
            | "FR"
            | "GR"
            | "HR"
            | "HU"
            | "IE"
            | "IT"
            | "LT"
            | "LU"
            | "LV"
            | "MT"
            | "NL"
            | "PL"
            | "PT"
            | "RO"
            | "SE"
            | "SI"
            | "SK"
            | "IS"
            | "LI"
            | "NO"
    )
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
        "NO_LONGER_PLACED_ON_THE_MARKET" | "NO_LONGER_ON_THE_MARKET" => {
            "NO_LONGER_PLACED_ON_MARKET"
        }
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
        "CST45" => "COLOUR", // BMS 3.1.35: COLOUR is now in GS1 clinicalSizeTypeCode (issue #39)
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
        "CST56" => "BODY_WEIGHT_KG", // BMS 3.1.35: BODY_WEIGHT_KG is now in GS1 clinicalSizeTypeCode (issue #39)
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

/// EUDAMED clinicalSize.text → GS1 ClinicalSizeCharacteristicsCode (BMS 3.1.35).
///
/// 35 allowed values per GS1 UDI Connector Profile Apr 2026 V1.1:
/// PASSIVE, ACTIVE, STRAIGHT, ANGLED, J-TIP, SOFT_STRAIGHT, STIFF_STRAIGHT,
/// SOFT_ANGLED, STIFF_ANGLED, STIFF_J-TIP, MINI, SACRAL, MULTISHAPE, HEEL,
/// CONTOUR, SQUARE, RECTANGULAR, BELT, CONVEX, STANDARD, CONVEX_LIGHT,
/// CONCAVE, FLAT, EXTRA_SMALL, SMALL, MEDIUM, LARGE, EXTRA_LARGE, NEONATE,
/// INFANT, CHILD, ADULT, LEFT, RIGHT, CURVED.
///
/// EUDAMED encodes characteristic descriptors as free-text in `clinicalSize.text`
/// when `precision="text"`. Map common variants (case-insensitive, allowing
/// abbreviations like S/M/L/XS/XL) to the GS1 code list. Returns None when
/// no characteristic match is found — caller keeps the text in
/// ClinicalSizeValueText as fallback.
///
/// Issue #39 (BMS 3.1.35).
pub fn text_to_characteristic_code(text: &str) -> Option<&'static str> {
    let t = text.trim().to_ascii_lowercase();
    Some(match t.as_str() {
        // Size abbreviations + full forms
        "xs" | "extra small" | "extra_small" | "extra-small" => "EXTRA_SMALL",
        "s" | "small" => "SMALL",
        "m" | "medium" => "MEDIUM",
        "l" | "large" => "LARGE",
        "xl" | "extra large" | "extra_large" | "extra-large" => "EXTRA_LARGE",
        "mini" => "MINI",
        // Patient age groups
        "neonate" => "NEONATE",
        "infant" => "INFANT",
        "child" => "CHILD",
        "adult" => "ADULT",
        // Body side
        "left" => "LEFT",
        "right" => "RIGHT",
        // Tip / orientation
        "active" => "ACTIVE",
        "passive" => "PASSIVE",
        "straight" => "STRAIGHT",
        "angled" => "ANGLED",
        "curved" => "CURVED",
        "j-tip" | "j tip" | "jtip" => "J-TIP",
        "soft straight" | "soft_straight" => "SOFT_STRAIGHT",
        "stiff straight" | "stiff_straight" => "STIFF_STRAIGHT",
        "soft angled" | "soft_angled" => "SOFT_ANGLED",
        "stiff angled" | "stiff_angled" => "STIFF_ANGLED",
        "stiff j-tip" | "stiff_j-tip" | "stiff jtip" => "STIFF_J-TIP",
        // Stoma / appliance shapes
        "sacral" => "SACRAL",
        "multishape" => "MULTISHAPE",
        "heel" => "HEEL",
        "contour" => "CONTOUR",
        "square" => "SQUARE",
        "rectangular" => "RECTANGULAR",
        "belt" => "BELT",
        "convex" => "CONVEX",
        "convex light" | "convex_light" => "CONVEX_LIGHT",
        "concave" => "CONCAVE",
        "flat" => "FLAT",
        "standard" => "STANDARD",
        _ => return None,
    })
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
        "MU999" => "", // "Other" unit — no valid UN/CEFACT mapping, skip
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

/// Issuing agency refdata code → GS1 identification type code
pub fn issuing_agency_to_type_code(agency: &str) -> &str {
    let suffix = agency.rsplit('.').next().unwrap_or(agency);
    match suffix {
        "gs1" => "GS1",
        "hibcc" => "HIBC",
        "iccbba" => "ICCBBA",
        "ifa" => "IFA",
        "eudamed" => "IFA", // EUDAMED-assigned DIs use IFA format (e.g. D-PD-F003MM)
        _ => "GS1",
    }
}

/// CMR substance type refdata suffix → GS1 CMR type code
/// e.g. "1a" → "CMR_1A", "1b" → "CMR_1B", "2" → "CMR_2"
pub fn cmr_type_to_gs1(code: &str) -> String {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    format!("CMR_{}", suffix.to_uppercase())
}

/// Multi-component refdata code → `MultiComponentDeviceTypeCode` (non-SPP path).
/// Used when `multiComponent.criterion=STANDARD` (FLD-UDID-12, MDR Art. 22(4):
/// "Procedure pack which is a device in itself"). The GDSN code list for
/// `MultiComponentDeviceTypeCode` per GS1 UDI Connector Profile Apr 2026 V1.1
/// is: DEVICE, PROCEDURE_PACK, SYSTEM, KIT. Issue #31 / #34.
pub fn multi_component_to_gs1(code: &str) -> &str {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    match suffix {
        "system" | "spp-system" => "SYSTEM",
        "procedure-pack" | "spp-procedure-pack" => "PROCEDURE_PACK",
        "kit" => "KIT",
        _ => "DEVICE",
    }
}

/// Multi-component refdata code → `SystemOrProcedurePackTypeCode` (SPP path).
/// Used when `multiComponent.criterion=SPP` (FLD-UDID-261, MDR Art. 22(1)/(3)).
/// The GDSN code list for `SystemOrProcedurePackTypeCode` per GS1 UDI Connector
/// Profile Apr 2026 V1.1 is **only** PROCEDURE_PACK and SYSTEM — DEVICE and KIT
/// are NOT valid in this attribute. Defaulting to DEVICE here used to trigger
/// G541 even after issue #34 (the wildcard arm). Now we default to
/// PROCEDURE_PACK (the conservative MDR Art. 22 fallback) and emit a warning,
/// because every SPP device should resolve to SYSTEM or PROCEDURE_PACK.
/// Issue #37.
pub fn spp_type_to_gs1(code: &str) -> &str {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    match suffix {
        "system" | "spp-system" => "SYSTEM",
        "procedure-pack" | "spp-procedure-pack" => "PROCEDURE_PACK",
        other => {
            eprintln!(
                "Warning: unexpected SPP multiComponent code suffix '{}' \
                 — defaulting to PROCEDURE_PACK (only SYSTEM/PROCEDURE_PACK \
                 valid for systemOrProcedurePackTypeCode per GS1 code list)",
                other
            );
            "PROCEDURE_PACK"
        }
    }
}

/// Risk class refdata code → GS1 risk class code
/// System 76 (MDR/IVDR Regulation): EU_CLASS_I/IIA/IIB/III, EU_CLASS_A/B/C/D
/// System 85 (MDD/AIMDD/IVDD Directive): EU_CLASS_I/IIA/IIB/III, AIMDD, IVDD_*
pub fn risk_class_refdata_to_gs1(code: &str) -> &str {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    match suffix {
        // MDR (system 76)
        "class-i" => "EU_CLASS_I",
        "class-iia" => "EU_CLASS_IIA",
        "class-iib" => "EU_CLASS_IIB",
        "class-iii" => "EU_CLASS_III",
        // IVDR (system 76)
        "class-a" => "EU_CLASS_A",
        "class-b" => "EU_CLASS_B",
        "class-c" => "EU_CLASS_C",
        "class-d" => "EU_CLASS_D",
        // IVDD old directive (system 85)
        "ivd-general" => "IVDD_GENERAL",
        "ivd-devices-self-testing" => "IVDD_DEVICES_SELF_TESTING",
        "ivd-annex-ii-list-a" => "IVDD_ANNEX_II_LIST_A",
        "ivd-annex-ii-list-b" => "IVDD_ANNEX_II_LIST_B",
        // AIMDD old directive (system 85)
        "aimdd" => "AIMDD",
        other => risk_class_to_gs1(other),
    }
}

/// Classification system code for risk class: "76" for MDR/IVDR, "85" for MDD/AIMDD/IVDD
pub fn risk_class_system_code(code: &str) -> &str {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    match suffix {
        "aimdd"
        | "ivd-general"
        | "ivd-devices-self-testing"
        | "ivd-annex-ii-list-a"
        | "ivd-annex-ii-list-b" => "85",
        _ => "76",
    }
}

/// Regulatory act from refdata risk class code
pub fn regulation_from_risk_class_refdata(code: &str) -> &str {
    let suffix = code.rsplit('.').next().unwrap_or(code);
    match suffix {
        "class-a" | "class-b" | "class-c" | "class-d" => "IVDR",
        "ivd-general"
        | "ivd-devices-self-testing"
        | "ivd-annex-ii-list-a"
        | "ivd-annex-ii-list-b" => "IVDD",
        "aimdd" => "AIMDD",
        _ => "MDR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characteristic_code_size_abbrevs() {
        assert_eq!(text_to_characteristic_code("S"), Some("SMALL"));
        assert_eq!(text_to_characteristic_code("m"), Some("MEDIUM"));
        assert_eq!(text_to_characteristic_code("L"), Some("LARGE"));
        assert_eq!(text_to_characteristic_code("XS"), Some("EXTRA_SMALL"));
        assert_eq!(text_to_characteristic_code("XL"), Some("EXTRA_LARGE"));
        assert_eq!(text_to_characteristic_code("Mini"), Some("MINI"));
        assert_eq!(text_to_characteristic_code("MINI"), Some("MINI"));
    }

    #[test]
    fn characteristic_code_orientations() {
        assert_eq!(text_to_characteristic_code("active"), Some("ACTIVE"));
        assert_eq!(text_to_characteristic_code("Passive"), Some("PASSIVE"));
        assert_eq!(text_to_characteristic_code("j-tip"), Some("J-TIP"));
        assert_eq!(
            text_to_characteristic_code("Stiff Straight"),
            Some("STIFF_STRAIGHT")
        );
        assert_eq!(text_to_characteristic_code("left"), Some("LEFT"));
    }

    #[test]
    fn characteristic_code_unknown_returns_none() {
        assert_eq!(text_to_characteristic_code("17.5mm"), None);
        assert_eq!(text_to_characteristic_code("foo bar"), None);
        assert_eq!(text_to_characteristic_code(""), None);
    }
}
