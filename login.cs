using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Data;
using System.Drawing;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using System.Windows.Forms;
using System.Security.Cryptography;
using System.Data.SqlClient;
using System.Text.RegularExpressions;
using System.Net.Mail;
using System.Net.NetworkInformation;
using System.Configuration;
using MySql.Data.MySqlClient;
using thepokerladder;

namespace pokerpractice
{
    public partial class login : Form
    {
        public login()
        {
            InitializeComponent();
        }
        string connStr = ConfigurationManager.ConnectionStrings["connstr"].ConnectionString;
        MySqlCommand command;
        string code = "";
        private void button1_Click(object sender, EventArgs e)
        {
            string pass = "", savedPasswordHash = "", username = textBox2.Text;
            bool active = false, validPass = true, validUser = Regex.IsMatch(username, @"^.*([A-Za-z\d]){3,12}$"); // turn this on for relase Regex.IsMatch(logPass, @"^(?=.*[a-z])(?=.*[A-Z])(?=.*\d)(?=.*[@$!%*?&])[A-Za-z\d@$!%*?&]{8,16}$")
            Ping ping = new Ping();
            PingReply pingReply = ping.Send("1.1.1.1");
            if (validPass && validUser && pingReply.Status == IPStatus.Success)
            {
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                        SELECT c.UserActive
                        FROM ranks AS c
                        WHERE c.UserID = @user";
                        command.Parameters.AddWithValue("@user", username);
                        conn.Open();
                        using (var reader = command.ExecuteReader())
                        {
                            while (reader.Read())
                            {
                                active = reader.GetBoolean(0);
                            }
                        }
                    }
                }
                if (!active)
                {
                    command = new MySqlCommand("");
                    using (var conn = new MySqlConnection(connStr))
                    {
                        using (command = conn.CreateCommand())
                        {
                            command.CommandText = @"
                            SELECT c.PassHash
                            FROM users AS c
                            WHERE c.UserID = @user";
                            command.Parameters.AddWithValue("@user", username);
                            conn.Open();
                            using (var reader = command.ExecuteReader())
                            {
                                while (reader.Read())
                                {
                                    savedPasswordHash = reader.GetString(0);
                                }
                            }
                        }
                    }

                    bool correctPass = true;
                    byte[] hashBytes = Convert.FromBase64String(savedPasswordHash);
                    byte[] salt = new byte[16];
                    Array.Copy(hashBytes, 0, salt, 0, 16);
                    var pbkdf2 = new Rfc2898DeriveBytes(pass, salt, 10000);
                    byte[] hash = pbkdf2.GetBytes(20);
                    for (int i = 0; i < 20; i++)
                    {
                        if (hashBytes[i + 16] != hash[i])
                            correctPass = false;
                    }
                    if (!correctPass)
                        MessageBox.Show("Incorrect password");
                    else
                    {
                        if (checkBox1.Checked)
                        {
                            thepokerladder.Properties.Settings.Default.userName = textBox2.Text;
                            thepokerladder.Properties.Settings.Default.userPass = textBox1.Text;
                            thepokerladder.Properties.Settings.Default.Save();
                        }
                        else
                        {
                            thepokerladder.Properties.Settings.Default.userName = "";
                            thepokerladder.Properties.Settings.Default.userPass = "";
                            thepokerladder.Properties.Settings.Default.Save();
                        }
                        this.Hide();
                        search frm = new search(username);
                        frm.ShowDialog();
                        this.Close();
                    }
                }
                else
                {
                    MessageBox.Show("User already logged in");
                }                
            }
            else
            {
                string invalidString = "";
                textBox4.Text = "";
                if (!validUser)
                {
                    invalidString += "Invalid username";
                }
                if (!validPass)
                {
                    invalidString += " Invalid password";
                }
                invalidString += ".";
                MessageBox.Show(invalidString);
            }
        }

        public bool IsValid(string emailaddress)
        {
            try
            {
                MailAddress address = new MailAddress(emailaddress);
                return true;
            }
            catch (FormatException)
            {
                return false;
            }
        }
        string email = "", passwordHash = "", username = "";
        private void button3_Click(object sender, EventArgs e)
        {
            string previous = "", invalidString = "", pass = "";
            username = textBox3.Text;
            email = textBox5.Text;
            bool validPass = Regex.IsMatch(pass, @"^(?=.*[a-z])(?=.*[A-Z])(?=.*\d)(?=.*[@$!%*?&])[A-Za-z\d@$!%*?&]{8,16}$"), validUser = Regex.IsMatch(username, @"^.*([A-Za-z\d]){3,12}$"), validEmail = IsValid(email);
            validPass = true;
            Ping ping = new Ping();
            PingReply pingReply = ping.Send("1.1.1.1");
            if (pingReply.Status == IPStatus.Success && validUser && validPass && validEmail)
            {
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                        SELECT c.UserID, c.Email
                        FROM users AS c
                        WHERE c.UserID = @user"; //removed collation
                        command.Parameters.AddWithValue("@user", username);
                        conn.Open();
                        using (var reader = command.ExecuteReader())
                        {
                            while (reader.Read())
                            {
                                previous = reader.GetString(0) + reader.GetString(1);
                            }
                        }
                    }
                }
                if (String.IsNullOrEmpty(previous))
                {
                    byte[] salt;
                    new RNGCryptoServiceProvider().GetBytes(salt = new byte[16]);
                    var pbkdf2 = new Rfc2898DeriveBytes(pass, salt, 10000);
                    byte[] hash = pbkdf2.GetBytes(20);
                    byte[] hashBytes = new byte[36];
                    Array.Copy(salt, 0, hashBytes, 0, 16);
                    Array.Copy(hash, 0, hashBytes, 16, 20);
                    passwordHash = Convert.ToBase64String(hashBytes);
                    code = codeGen();
                    var mail = new EmailRegister();
                    mail.Send(code, email);
                    groupBox1.Visible = false;
                    groupBox2.Visible = false;
                    groupBox3.Visible = true;
                }
                else
                {
                    MessageBox.Show("User already registered, try again");
                }
            }
            else
            {
                textBox4.Text = "";
                if (!validEmail)
                {
                    invalidString = "Invalid email";
                }
                if (!validUser)
                {
                    invalidString += " Invalid username";
                }
                if (!validPass)
                {
                    invalidString += " Invalid password";
                }
                invalidString += ".";
                MessageBox.Show(invalidString);
            }
        }

        private string codeGen()
        {
            var chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            var stringChars = new char[20];
            var random = new Random();
            for (int i = 0; i < 20; i++)
            {
                stringChars[i] = chars[random.Next(62)];
            }
            return new String(stringChars);
        }

        private void button2_Click(object sender, EventArgs e)
        {
            groupBox1.Visible = true;
            groupBox2.Visible = false;
        }

        private void button4_Click(object sender, EventArgs e)
        {
            groupBox1.Visible = false;
            groupBox2.Visible = true;
        }

        private void textBox1_KeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Enter)
            {
                button1_Click(this, new EventArgs());
            }
        }

        private void textBox2_KeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Enter)
            {
                button1_Click(this, new EventArgs());
            }
        }

        private void textBox3_KeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Enter)
            {
                button3_Click(this, new EventArgs());
            }
        }

        private void textBox4_KeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Enter)
            {
                button3_Click(this, new EventArgs());
            }
        }

        private void textBox5_KeyDown(object sender, KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Enter)
            {
                button3_Click(this, new EventArgs());
            }
        }

        private void login_Load(object sender, EventArgs e)
        {
            //https://wyday.com/wybuild/help/automatic-updates/
            groupBox2.Visible = true;
            if (!String.IsNullOrEmpty(thepokerladder.Properties.Settings.Default.userName))
            {
                textBox2.Text = thepokerladder.Properties.Settings.Default.userName;
                textBox1.Text = thepokerladder.Properties.Settings.Default.userPass;
                checkBox1.Checked = true;
            }
        }

        private void button6_Click(object sender, EventArgs e)
        {
            groupBox2.Visible = false;
            groupBox1.Visible = true;
            groupBox3.Visible = false;
        }

        private void button5_Click(object sender, EventArgs e)
        {
            if (!textBox6.Text.Equals(code))
                label9.Visible = true;
            else
            {
                command = new MySqlCommand("");
                using (var conn = new MySqlConnection(connStr))
                {
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                                INSERT INTO users (users.UserID, users.PassHash, users.Email)
                                VALUES (@userID, @ph, @email);";
                        command.Parameters.AddWithValue("@email", email);
                        command.Parameters.AddWithValue("@ph", passwordHash);
                        command.Parameters.AddWithValue("@userID", username);
                        conn.Open();
                        command.ExecuteNonQuery();
                        conn.Close();
                    }
                    using (command = conn.CreateCommand())
                    {
                        command.CommandText = @"
                                INSERT INTO ranks (ranks.UserID, ranks.UserRank)
                                VALUES (@userID, @rank);";
                        command.Parameters.AddWithValue("@userID", username);
                        command.Parameters.AddWithValue("@rank", 1000);
                        conn.Open();
                        command.ExecuteNonQuery();
                        conn.Close();
                    }
                }
                groupBox1.Visible = false;
                groupBox2.Visible = true;
                groupBox3.Visible = false;
            }
        }
    }
}
