def compare_version(new_version, current_version) -> bool:
    new_version_parts = new_version.split(".")
    current_version_parts = current_version.split(".")

    for i in range(len(new_version_parts)):
        if int(new_version_parts[i]) > int(current_version_parts[i]):
            return True
        elif int(new_version_parts[i]) < int(current_version_parts[i]):
            return False

    return False
