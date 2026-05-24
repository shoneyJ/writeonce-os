// confirm.rs — destructive-operation safety prompts.

use anyhow::Result;
use std::io::{BufRead, Write};

use crate::detect::UsbDevice;

/// Print device + ask user to type "yes" exactly. Default to no.
pub fn confirm_wipe(device: &UsbDevice) -> Result<bool> {
    println!();
    println!("============================================================");
    println!(" CONFIRM DESTRUCTIVE OPERATION");
    println!("============================================================");
    println!(" Target:    {}", device.path.display());
    println!(" Vendor:    {}", device.vendor);
    println!(" Model:     {}", device.model);
    println!(" Size:      {:.2} GB ({} bytes)", device.size_gb(), device.size_bytes);
    println!(" Removable: {}", device.removable);
    println!();
    println!(" The ENTIRE contents of {} will be erased.", device.path.display());
    println!(" Type \"yes\" exactly (no quotes) to continue, anything else aborts.");
    print!(" > ");
    std::io::stdout().flush()?;

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim() == "yes")
}
